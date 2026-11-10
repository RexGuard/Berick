/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Berick: a web browser built on the Servo engine.

mod browser;
mod gui;
mod keyutils;
mod location;

use std::error::Error;
use std::rc::Rc;

use log::warn;
use url::Url;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::window::WindowId;

use crate::browser::Browser;

const HOME_PAGE: &str = "https://servo.org/";

fn main() -> Result<(), Box<dyn Error>> {
    // Logging is set up by Servo (`Servo::setup_logging`), controlled by RUST_LOG.
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("Failed to install the TLS crypto provider");

    // `berick <url or search>` opens that instead of the home page.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let initial_url = Some(args.join(" "))
        .filter(|input| !input.trim().is_empty())
        .and_then(|input| location::resolve(&input, location::DEFAULT_SEARCH_URL))
        .unwrap_or_else(|| Url::parse(HOME_PAGE).expect("home page is a valid URL"));

    let event_loop = EventLoop::<WakeUp>::with_user_event().build()?;
    let mut app = App {
        proxy: event_loop.create_proxy(),
        initial_url,
        browser: None,
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}

/// Sent by Servo (from any thread) when it has work for the main thread.
#[derive(Debug)]
struct WakeUp;

#[derive(Clone)]
struct Waker(EventLoopProxy<WakeUp>);

impl servo::EventLoopWaker for Waker {
    fn clone_box(&self) -> Box<dyn servo::EventLoopWaker> {
        Box::new(self.clone())
    }

    fn wake(&self) {
        if let Err(error) = self.0.send_event(WakeUp) {
            warn!("Failed to wake the event loop: {error:?}");
        }
    }
}

struct App {
    proxy: EventLoopProxy<WakeUp>,
    initial_url: Url,
    browser: Option<Rc<Browser>>,
}

impl ApplicationHandler<WakeUp> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.browser.is_none() {
            let waker = Box::new(Waker(self.proxy.clone()));
            self.browser = Some(Browser::new(event_loop, waker, self.initial_url.clone()));
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, _event: WakeUp) {
        if let Some(browser) = &self.browser {
            browser.spin();
            if browser.should_close() {
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if let Some(browser) = &self.browser {
            browser.spin();
            browser.handle_window_event(event_loop, event);
            if browser.should_close() {
                event_loop.exit();
            }
        }
    }
}
