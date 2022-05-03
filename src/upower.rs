use zbus;
use upower_dbus;
use std::sync::{Arc, RwLock, mpsc::{Sender, Receiver}};
use std::sync::mpsc::SyncSender;
use crate::PowerState;

pub struct PowerReporter {
    sender: Sender<()>,
    status: Arc<RwLock<Option<PowerState>>>,
}

pub struct PowerReceiver {
    receiver: Receiver<()>,
    status: Arc<RwLock<Option<PowerState>>>,
}

pub struct UPowerMonitor {
    reporter: PowerReporter,

    dbus: zbus::Connection,
}

macro_rules! catch {
    ($expr:block) => {
        (|| { $expr })()
    };
}

pub fn spawn_upower(reporter: PowerReporter) -> anyhow::Result<()> {
    let (start_send, start_receive) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(|| {
        let failure = upower_run(reporter, &start_send);;
        if failure.is_err() {
            start_send.send(failure);
        }
    });

    start_receive.recv()?
}

async fn upower_run(reporter: PowerReporter, start_send: &SyncSender<anyhow::Result<()>>) {
    let dbus = zbus::blocking::Connection::system()?;
    let display_device_path = upower_dbus::UPowerProxyBlocking::new(&connection).await?
        .get_display_device().await?;
    zbus::blocking::fdo::PropertiesProxy::builder(&dbus)
        

    let display_proxy = upower_dbus::DeviceProxyBlocking::builder()
        .path(display_device_path)?
        .build().await?;

    start_send.send(Ok(())).unwrap();
}

impl UPowerMonitor {
    fn spawn(reporter: PowerReporter) -> anyhow::Result<Self> {

        futures::executor::block_on(async {
            let dbus = zbus::Connection::system().await?;

            Ok(Self {
                reporter,
                dbus,
            })
        })
    }

}

