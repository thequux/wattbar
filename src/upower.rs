use std::collections::HashMap;
use crate::PowerState;
use std::sync::mpsc::SyncSender;
use std::sync::{
    mpsc::{Receiver, Sender},
    Arc, RwLock,
};
use upower_dbus;

use calloop::channel::Sender as CalloopSender;
use upower_dbus::BatteryState;
use zbus;
use zbus::zvariant::OwnedValue;

pub struct PowerReporter {
    pub sender: CalloopSender<()>,
    pub status: Arc<RwLock<Option<PowerState>>>,
}

pub struct PowerReceiver {
    receiver: Receiver<()>,
    status: Arc<RwLock<Option<PowerState>>>,
}

macro_rules! catch {
    ($expr:block) => {
        (|| $expr)()
    };
}

pub fn spawn_mock(reporter: PowerReporter) -> anyhow::Result<()> {
    std::thread::spawn(move || {
        *reporter.status.write().unwrap() = Some(PowerState{
            level: 0.0,
            charging: false,
            time_remaining: 0.0,
        });
        let mut fill = 0u32;
       loop {
           std::thread::sleep(std::time::Duration::from_millis(10));
           {
               let mut lock = reporter.status.write().unwrap();
               fill = (fill + 1) & 0x1FF;
               lock.as_mut().unwrap().level = (fill as f32) / 512.0f32;
           };
           reporter.sender.send(()).unwrap();
       }
    });
    Ok(())
}

pub fn spawn_upower(reporter: PowerReporter) -> anyhow::Result<()> {
    let (start_send, start_receive) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let failure = upower_run(reporter, &start_send);
        if failure.is_err() {
            start_send.send(failure);
        }
    });

    start_receive.recv()?
}

fn upower_update(reporter: &PowerReporter, properties: &HashMap<String, OwnedValue>) {
    {
        let mut status = reporter.status.write().unwrap();
        let battery_state = upower_dbus::BatteryState::try_from(properties["State"].clone()).unwrap();
        let charging = match battery_state {
            // fully enumerate the options in case a new one is added.
            BatteryState::Charging |
            BatteryState::FullyCharged |
            BatteryState::PendingCharge => true,
            BatteryState::Empty |
            BatteryState::Discharging |
            BatteryState::PendingDischarge |
            BatteryState::Unknown => false,
        };
        *status = Some(PowerState {
            level: f64::try_from(&properties["Percentage"]).unwrap() as f32 / 100.0,
            charging,
            time_remaining: if charging {
                i64::try_from(&properties["TimeToFull"]).unwrap()
            } else {
                i64::try_from(&properties["TimeToEmpty"]).unwrap()
            } as f32
        })
    }
    // Notify listeners
    reporter.sender.send(()).ok();
}

fn upower_run(
    reporter: PowerReporter,
    start_send: &SyncSender<anyhow::Result<()>>,
) -> anyhow::Result<()> {
    let dbus = zbus::blocking::Connection::system()?;
    let display_device_path = upower_dbus::UPowerProxyBlocking::new(&dbus)?.get_display_device()?;
    let display_proxy : zbus::blocking::fdo::PropertiesProxy = zbus::blocking::fdo::PropertiesProxy::builder(&dbus)
        .destination("org.freedesktop.UPower")?
        .path(display_device_path)?
        .cache_properties(zbus::CacheProperties::No)
        .build()?;

    let mut prop_changed_iterator = display_proxy.receive_properties_changed()?;

    let device_interface_name = zbus::names::InterfaceName::from_static_str("org.freedesktop.UPower.Device").unwrap();

    let mut properties: HashMap<String, OwnedValue> = display_proxy.get_all(device_interface_name.clone())?;

    upower_update(&reporter, &properties);
    start_send.send(Ok(())).unwrap();
    for signal in prop_changed_iterator {
        let args = signal.args().expect("Invalid signal arguments");
        if args.interface_name != device_interface_name {
            continue
        }

        for (name, value) in args.changed_properties {
            properties.get_mut(name).map(|vp| *vp = value.into());
        }
        
        // Update reporter
        upower_update(&reporter, &properties);
    }

    // TODO: actually watch for events
    Ok(())
}

