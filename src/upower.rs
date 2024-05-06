use crate::PowerState;
use std::collections::HashMap;
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, RwLock};
use upower_dbus;

use crate::theme::ChargeState;
use calloop::channel::Sender as CalloopSender;
use upower_dbus::BatteryState;
use zbus;
use zbus::zvariant::OwnedValue;

pub struct PowerReporter {
    pub sender: CalloopSender<()>,
    pub status: Arc<RwLock<Option<PowerState>>>,
}

pub fn spawn_mock(reporter: PowerReporter) -> anyhow::Result<()> {
    std::thread::spawn(move || {
        *reporter.status.write().unwrap() = Some(PowerState {
            level: 0.0,
            state: ChargeState::Discharging,
            time_remaining: 0.0,
        });
        let mut fill = 0u32;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(10));
            {
                let mut lock = reporter.status.write().unwrap();
                fill = (fill + 1) % 0x1FF;
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
            start_send.send(failure).unwrap();
        }
    });

    start_receive.recv()?
}

fn upower_update(reporter: &PowerReporter, properties: &HashMap<String, OwnedValue>) {
    {
        let mut status = reporter.status.write().unwrap();
        let battery_state =
            upower_dbus::BatteryState::try_from(properties["State"].clone()).unwrap();
        let state = match battery_state {
            BatteryState::Unknown => ChargeState::NoCharge,
            BatteryState::Charging => ChargeState::Charging,
            BatteryState::Discharging => ChargeState::Discharging,
            BatteryState::Empty => ChargeState::NoCharge,
            BatteryState::FullyCharged => ChargeState::NoCharge,
            BatteryState::PendingCharge => ChargeState::NoCharge,
            BatteryState::PendingDischarge => ChargeState::Discharging, // not sure about this one
        };
        *status = Some(PowerState {
            level: f64::try_from(&properties["Percentage"]).unwrap() as f32 / 100.0,
            state,
            time_remaining: match state {
                ChargeState::Charging => i64::try_from(&properties["TimeToFull"]).unwrap(),
                ChargeState::NoCharge => 0,
                ChargeState::Discharging => i64::try_from(&properties["TimeToEmpty"]).unwrap(),
            } as f32,
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
    let display_device = upower_dbus::UPowerProxyBlocking::new(&dbus)?.get_display_device()?;
    let display_proxy: zbus::blocking::fdo::PropertiesProxy =
        zbus::blocking::fdo::PropertiesProxy::builder(&dbus)
            .destination("org.freedesktop.UPower")?
            .path(display_device.path())?
            .cache_properties(zbus::CacheProperties::No)
            .build()?;

    let prop_changed_iterator = display_proxy.receive_properties_changed()?;

    let device_interface_name =
        zbus::names::InterfaceName::from_static_str("org.freedesktop.UPower.Device").unwrap();

    let mut properties: HashMap<String, OwnedValue> =
        display_proxy.get_all(device_interface_name.clone())?;

    upower_update(&reporter, &properties);
    start_send.send(Ok(())).unwrap();
    for signal in prop_changed_iterator {
        let args = signal.args().expect("Invalid signal arguments");
        if args.interface_name != device_interface_name {
            continue;
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
