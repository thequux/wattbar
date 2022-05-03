use crate::PowerState;
use std::sync::mpsc::SyncSender;
use std::sync::{
    mpsc::{Receiver, Sender},
    Arc, RwLock,
};
use upower_dbus;

use calloop::channel::Sender as CalloopSender;
use zbus;

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

fn upower_run(
    reporter: PowerReporter,
    start_send: &SyncSender<anyhow::Result<()>>,
) -> anyhow::Result<()> {
    let dbus = zbus::blocking::Connection::system()?;
    let display_device_path = upower_dbus::UPowerProxyBlocking::new(&dbus)?.get_display_device()?;
    let display_proxy = zbus::blocking::fdo::PropertiesProxy::builder(&dbus)
        .destination("org.freedesktop.UPower")?
        .path(display_device_path)?
        .cache_properties(zbus::CacheProperties::No)
        .build()?;

    start_send.send(Ok(())).unwrap();

    // TODO: actually watch for events
    Ok(())
}

