use super::{Finish, Request};
use crate::app::use_app;
use anyhow::{anyhow, Result};
use clippy_platform::clipboard::{
    foreground::{Publisher, Ticket},
    Offer,
};
use slint::{
    winit_030::{
        winit::{
            raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawWindowHandle},
            window::Window,
        },
        WinitWindowAccessor,
    },
    Timer,
};
use std::{cell::RefCell, time::Duration};

struct Host {
    publisher: Publisher,
    finish: Option<Finish>,
}

thread_local! {
    static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
}

pub(crate) fn attach(window: &Window) {
    HOST.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            let Ok(display) = window.display_handle() else {
                return;
            };
            // Wayland 的处理器在 Winit 销毁显示连接前清理 HOST。
            match unsafe { Publisher::attach(display) } {
                Ok(publisher) => {
                    *slot = Some(Host {
                        publisher,
                        finish: None,
                    })
                }
                Err(error) => eprintln!("Foreground clipboard unavailable: {error:#}"),
            }
        }
    });
}

pub(super) fn ticket(window: &slint::Window) -> Option<Ticket> {
    let surface =
        window.with_winit_window(|native| match native.window_handle().ok()?.as_raw() {
            RawWindowHandle::Wayland(window) => Some(window.surface),
            _ => None,
        })??;
    HOST.with(|cell| {
        let mut slot = cell.borrow_mut();
        let host = slot.as_mut()?;
        host.publisher.dispatch().ok()?;
        host.publisher.ticket(surface)
    })
}

pub(crate) fn publish(ticket: Ticket, result: Result<Offer>, finish: Finish) {
    let offer = match result {
        Ok(offer) => offer,
        Err(error) => return finish(Err(error)),
    };
    let mut completion = Some(finish);
    let result = HOST.with(|cell| {
        let mut slot = cell.borrow_mut();
        let host = slot
            .as_mut()
            .ok_or_else(|| anyhow!("Focus the Wayland window before copying"))?;
        anyhow::ensure!(host.finish.is_none(), "Clipboard is busy. Try again.");
        host.publisher.publish(ticket, offer)?;
        host.finish = completion.take();
        Ok(())
    });
    if let Err(error) = result {
        return completion.expect("Unsubmitted clipboard callback")(Err(error));
    }
    Timer::single_shot(Duration::from_millis(1050), service);
}

pub(crate) fn service() {
    let work = HOST.with(|cell| {
        let mut slot = cell.borrow_mut();
        let host = slot.as_mut()?;
        match host.publisher.dispatch() {
            Ok(()) => Some((
                host.publisher.take_transfers(),
                host.publisher
                    .take_result()
                    .map(|result| (host.finish.take(), result)),
            )),
            Err(error) => {
                let finish = host.finish.take();
                slot.take();
                Some((Vec::new(), Some((finish, Err(error)))))
            }
        }
    });
    if let Some((transfers, result)) = work {
        if !transfers.is_empty() {
            use_app(|app| {
                for transfer in transfers {
                    if let Err(error) = app.clipboard.submit(Request::ForegroundTransfer(transfer))
                    {
                        eprintln!("Could not queue clipboard data: {error:#}");
                    }
                }
            });
        }
        if let Some((Some(finish), result)) = result {
            finish(result);
        }
    }
}

pub(crate) fn shutdown() {
    HOST.with(|cell| {
        cell.take();
    });
}
