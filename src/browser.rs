/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! One Berick window: the Servo instance, the page (WebView), the toolbar, and the glue
//! that routes input to the right one.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use euclid::{Point2D, Rect, Scale};
use keyboard_types::{Key, Modifiers, NamedKey, ShortcutMatcher};
use log::{info, warn};
use servo::{
    CreateNewWebViewRequest, Cursor, DevicePixel, InputEvent, KeyboardEvent, LoadStatus,
    MouseButton as ServoMouseButton, MouseButtonAction, MouseButtonEvent, MouseLeftViewportEvent,
    MouseMoveEvent, RenderingContext, Servo, ServoBuilder, Theme, WebView, WebViewBuilder,
    WebViewDelegate, WheelDelta, WheelEvent, WheelMode, WindowRenderingContext,
};
use url::Url;
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::{CursorIcon, Window};

use crate::gui::{Command, Gui};
use crate::keyutils::{CMD_OR_ALT, CMD_OR_CONTROL, keyboard_event_from_winit};
use crate::location;

/// Pixels to scroll per mouse wheel "line".
const LINE_HEIGHT: f32 = 76.0;
const LINE_WIDTH: f32 = 76.0;

pub struct Browser {
    me: Weak<Browser>,
    window: Window,
    servo: Servo,
    window_rendering_context: Rc<WindowRenderingContext>,
    rendering_context: Rc<servo::OffscreenRenderingContext>,
    gui: RefCell<Gui>,
    /// Pages in this window. The last one is visible. Pop-ups (`window.open`,
    /// `target=_blank`) are pushed on top until real tabs land.
    webviews: RefCell<Vec<WebView>>,
    modifiers: Cell<ModifiersState>,
    /// Mouse position relative to the top-left of the page, in device pixels.
    webview_mouse_point: Cell<Point2D<f32, DevicePixel>>,
    /// Mouse position in the window, in logical pixels. `None` until the mouse moves.
    window_mouse_y: Cell<Option<f32>>,
    search_url: String,
}

impl Browser {
    pub fn new(
        event_loop: &ActiveEventLoop,
        waker: Box<dyn servo::EventLoopWaker>,
        initial_url: Url,
    ) -> Rc<Self> {
        let attributes = Window::default_attributes()
            .with_title("Berick")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 800.0))
            .with_visible(false);
        let window = event_loop
            .create_window(attributes)
            .expect("Failed to create window");

        let display_handle = event_loop
            .display_handle()
            .expect("Failed to get display handle");
        let window_handle = window.window_handle().expect("Failed to get window handle");
        let inner_size = window.inner_size();
        let window_rendering_context = Rc::new(
            WindowRenderingContext::new(display_handle, window_handle, inner_size)
                .expect("Could not create a rendering context for the window"),
        );
        window_rendering_context
            .make_current()
            .expect("Could not make the window rendering context current");
        let rendering_context = Rc::new(window_rendering_context.offscreen_context(inner_size));

        let servo = ServoBuilder::default().event_loop_waker(waker).build();
        servo.setup_logging();

        let gui = Gui::new(
            event_loop,
            rendering_context.clone(),
            initial_url.to_string(),
        );
        window.set_visible(true);

        let browser = Rc::new_cyclic(|me| Browser {
            me: me.clone(),
            window,
            servo,
            window_rendering_context,
            rendering_context,
            gui: RefCell::new(gui),
            webviews: Default::default(),
            modifiers: Cell::new(ModifiersState::empty()),
            webview_mouse_point: Cell::new(Point2D::zero()),
            window_mouse_y: Cell::new(None),
            search_url: location::DEFAULT_SEARCH_URL.to_string(),
        });

        let webview = WebViewBuilder::new(&browser.servo, browser.rendering_context.clone())
            .url(initial_url)
            .hidpi_scale_factor(browser.hidpi_scale())
            .delegate(browser.clone())
            .build();
        webview.show();
        webview.set_focused(true);
        browser.webviews.borrow_mut().push(webview);

        browser
    }

    fn hidpi_scale(&self) -> Scale<f32, servo::DeviceIndependentPixel, DevicePixel> {
        Scale::new(self.window.scale_factor() as f32)
    }

    fn active_webview(&self) -> Option<WebView> {
        self.webviews.borrow().last().cloned()
    }

    /// Let Servo do pending work. Called whenever Servo wakes the event loop.
    pub fn spin(&self) {
        self.servo.spin_event_loop();
        if self
            .gui
            .borrow_mut()
            .update_from_webview(self.active_webview().as_ref())
        {
            self.window.request_redraw();
        }
    }

    pub fn should_close(&self) -> bool {
        self.webviews.borrow().is_empty()
    }

    fn run_commands(&self) {
        let commands = self.gui.borrow_mut().take_commands();
        let Some(webview) = self.active_webview() else {
            return;
        };
        for command in commands {
            match command {
                Command::Back => {
                    webview.go_back(1);
                },
                Command::Forward => {
                    webview.go_forward(1);
                },
                Command::Reload => webview.reload(),
                Command::Go(input) => match location::resolve(&input, &self.search_url) {
                    Some(url) => {
                        info!("Loading {url}");
                        webview.load(url);
                        // Hand the keyboard back to the page.
                        self.gui.borrow().surrender_focus();
                        webview.set_focused(true);
                    },
                    None => warn!("Nothing to load for {input:?}"),
                },
            }
        }
    }

    fn toolbar_height_device_pixels(&self) -> f32 {
        self.gui.borrow().toolbar_height() * self.hidpi_scale().get()
    }

    fn is_over_toolbar(&self) -> bool {
        self.window_mouse_y
            .get()
            .is_none_or(|y| y < self.gui.borrow().toolbar_height())
    }

    /// Browser shortcuts that the page never sees. Returns true if the key was handled.
    fn handle_shortcut(&self, webview: &WebView, event: &KeyboardEvent) -> bool {
        let mut handled = true;
        ShortcutMatcher::from_event(event.event.clone())
            .shortcut(CMD_OR_CONTROL, 'L', || {
                self.gui.borrow_mut().focus_location_bar()
            })
            .optional_shortcut(!cfg!(target_os = "macos"), Modifiers::ALT, 'D', || {
                self.gui.borrow_mut().focus_location_bar()
            })
            .shortcut(Modifiers::empty(), Key::Named(NamedKey::F6), || {
                self.gui.borrow_mut().focus_location_bar()
            })
            .shortcut(CMD_OR_CONTROL, 'R', || webview.reload())
            .shortcut(Modifiers::empty(), Key::Named(NamedKey::F5), || {
                webview.reload()
            })
            .shortcut(CMD_OR_ALT, Key::Named(NamedKey::ArrowLeft), || {
                webview.go_back(1);
            })
            .shortcut(CMD_OR_ALT, Key::Named(NamedKey::ArrowRight), || {
                webview.go_forward(1);
            })
            .optional_shortcut(cfg!(target_os = "macos"), CMD_OR_CONTROL, '[', || {
                webview.go_back(1);
            })
            .optional_shortcut(cfg!(target_os = "macos"), CMD_OR_CONTROL, ']', || {
                webview.go_forward(1);
            })
            .shortcut(CMD_OR_CONTROL, '=', || zoom(webview, 1.1))
            .shortcut(CMD_OR_CONTROL, '+', || zoom(webview, 1.1))
            .shortcut(CMD_OR_CONTROL, '-', || zoom(webview, 1.0 / 1.1))
            .shortcut(CMD_OR_CONTROL, '0', || webview.set_page_zoom(1.0))
            .otherwise(|| handled = false);
        if handled {
            self.window.request_redraw();
        }
        handled
    }

    fn handle_keyboard(&self, webview: &WebView, event: &KeyEvent) {
        let keyboard_event = keyboard_event_from_winit(event, self.modifiers.get());
        if keyboard_event.event.state == keyboard_types::KeyState::Down
            && self.handle_shortcut(webview, &keyboard_event)
        {
            return;
        }
        webview.notify_input_event(InputEvent::Keyboard(keyboard_event));
    }

    fn handle_mouse_move(&self, webview: &WebView, position: PhysicalPosition<f64>) {
        let mut point = Point2D::new(position.x as f32, position.y as f32);
        point.y -= self.toolbar_height_device_pixels();
        let previous = self.webview_mouse_point.replace(point);

        let webview_rect: Rect<_, _> = webview.size().into();
        if !webview_rect.contains(point) {
            if webview_rect.contains(previous) {
                webview.notify_input_event(InputEvent::MouseLeftViewport(
                    MouseLeftViewportEvent::default(),
                ));
            }
            return;
        }
        webview.notify_input_event(InputEvent::MouseMove(MouseMoveEvent::new(point.into())));
    }

    fn handle_mouse_button(&self, webview: &WebView, button: MouseButton, state: ElementState) {
        let point = self.webview_mouse_point.get();
        let webview_rect: Rect<_, _> = webview.size().into();
        if !webview_rect.contains(point) {
            return;
        }
        let button = match button {
            MouseButton::Left => ServoMouseButton::Primary,
            MouseButton::Right => ServoMouseButton::Secondary,
            MouseButton::Middle => ServoMouseButton::Auxiliary,
            MouseButton::Back => ServoMouseButton::Back,
            MouseButton::Forward => ServoMouseButton::Forward,
            MouseButton::Other(value) => ServoMouseButton::Other(value),
        };
        let action = match state {
            ElementState::Pressed => MouseButtonAction::Down,
            ElementState::Released => MouseButtonAction::Up,
        };
        webview.notify_input_event(InputEvent::MouseButton(MouseButtonEvent::new(
            action,
            button,
            point.into(),
        )));
    }

    pub fn handle_window_event(&self, event_loop: &ActiveEventLoop, event: WindowEvent) {
        if let WindowEvent::Resized(new_size) = event {
            self.window_rendering_context.resize(new_size);
        }
        if matches!(
            event,
            WindowEvent::RedrawRequested | WindowEvent::Resized(_)
        ) {
            let webview = self.active_webview();
            let mut gui = self.gui.borrow_mut();
            gui.update_from_webview(webview.as_ref());
            gui.update(&self.window, webview.as_ref());
            gui.paint(&self.window);
            drop(gui);
            self.run_commands();
        }

        if let WindowEvent::CursorMoved { position, .. } = event {
            self.window_mouse_y
                .set(Some(position.y as f32 / self.hidpi_scale().get()));
        }

        let Some(webview) = self.active_webview() else {
            return;
        };

        // Mouse back/forward buttons work anywhere in the window.
        let mut consumed = false;
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                return;
            },
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Back,
                ..
            } => {
                webview.go_back(1);
                consumed = true;
            },
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Forward,
                ..
            } => {
                webview.go_forward(1);
                consumed = true;
            },
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                webview.set_hidpi_scale_factor(Scale::new(scale_factor as f32));
                self.window.request_redraw();
            },
            WindowEvent::MouseWheel { .. } | WindowEvent::MouseInput { .. }
                if !self.is_over_toolbar() =>
            {
                // Clicking or scrolling the page takes focus away from the address bar.
                self.gui.borrow().surrender_focus();
                webview.set_focused(true);
            },
            WindowEvent::KeyboardInput { .. } if !self.gui.borrow().has_keyboard_focus() => {
                // Keys go to the page unless the address bar has focus.
            },
            ref event => {
                let response = self.gui.borrow_mut().on_window_event(&self.window, event);
                if response.repaint && *event != WindowEvent::RedrawRequested {
                    self.window.request_redraw();
                }
                consumed = match event {
                    // The page also needs mouse moves to update hover state and leave events.
                    WindowEvent::CursorMoved { .. } => self.is_over_toolbar(),
                    _ => response.consumed,
                };
            },
        }
        if consumed {
            return;
        }

        match event {
            WindowEvent::KeyboardInput { event, .. } => self.handle_keyboard(&webview, &event),
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers.set(modifiers.state()),
            WindowEvent::CursorMoved { position, .. } => self.handle_mouse_move(&webview, position),
            WindowEvent::CursorLeft { .. } => {
                let webview_rect: Rect<_, _> = webview.size().into();
                if webview_rect.contains(self.webview_mouse_point.get()) {
                    webview.notify_input_event(InputEvent::MouseLeftViewport(
                        MouseLeftViewportEvent::default(),
                    ));
                }
            },
            WindowEvent::MouseInput { state, button, .. } => {
                self.handle_mouse_button(&webview, button, state)
            },
            WindowEvent::MouseWheel { delta, .. } => {
                let (x, y) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => {
                        ((x * LINE_WIDTH) as f64, (y * LINE_HEIGHT) as f64)
                    },
                    MouseScrollDelta::PixelDelta(delta) => (delta.x, delta.y),
                };
                let delta = WheelDelta {
                    x,
                    y,
                    z: 0.0,
                    mode: WheelMode::DeltaPixel,
                };
                webview.notify_input_event(InputEvent::Wheel(WheelEvent::new(
                    delta,
                    self.webview_mouse_point.get().into(),
                )));
            },
            WindowEvent::ThemeChanged(theme) => webview.notify_theme_change(match theme {
                winit::window::Theme::Light => Theme::Light,
                winit::window::Theme::Dark => Theme::Dark,
            }),
            _ => {},
        }
    }
}

fn zoom(webview: &WebView, factor: f32) {
    let zoom = (webview.page_zoom() * factor).clamp(0.25, 5.0);
    webview.set_page_zoom(zoom);
}

impl WebViewDelegate for Browser {
    fn notify_new_frame_ready(&self, _webview: WebView) {
        self.window.request_redraw();
    }

    fn notify_url_changed(&self, _webview: WebView, _url: Url) {
        self.window.request_redraw();
    }

    fn notify_page_title_changed(&self, webview: WebView, title: Option<String>) {
        if Some(webview.id()) != self.active_webview().map(|w| w.id()) {
            return;
        }
        let title = match title {
            Some(title) if !title.trim().is_empty() => format!("{} - Berick", title.trim()),
            _ => "Berick".to_string(),
        };
        self.window.set_title(&title);
    }

    fn notify_load_status_changed(&self, _webview: WebView, _status: LoadStatus) {
        self.window.request_redraw();
    }

    fn notify_history_changed(&self, _webview: WebView, _entries: Vec<Url>, _current: usize) {
        self.window.request_redraw();
    }

    fn notify_status_text_changed(&self, _webview: WebView, _status: Option<String>) {
        self.window.request_redraw();
    }

    fn notify_cursor_changed(&self, _webview: WebView, cursor: Cursor) {
        let icon = match cursor {
            Cursor::None => {
                self.window.set_cursor_visible(false);
                return;
            },
            Cursor::Default => CursorIcon::Default,
            Cursor::Pointer => CursorIcon::Pointer,
            Cursor::ContextMenu => CursorIcon::ContextMenu,
            Cursor::Help => CursorIcon::Help,
            Cursor::Progress => CursorIcon::Progress,
            Cursor::Wait => CursorIcon::Wait,
            Cursor::Cell => CursorIcon::Cell,
            Cursor::Crosshair => CursorIcon::Crosshair,
            Cursor::Text => CursorIcon::Text,
            Cursor::VerticalText => CursorIcon::VerticalText,
            Cursor::Alias => CursorIcon::Alias,
            Cursor::Copy => CursorIcon::Copy,
            Cursor::Move => CursorIcon::Move,
            Cursor::NoDrop => CursorIcon::NoDrop,
            Cursor::NotAllowed => CursorIcon::NotAllowed,
            Cursor::Grab => CursorIcon::Grab,
            Cursor::Grabbing => CursorIcon::Grabbing,
            Cursor::EResize => CursorIcon::EResize,
            Cursor::NResize => CursorIcon::NResize,
            Cursor::NeResize => CursorIcon::NeResize,
            Cursor::NwResize => CursorIcon::NwResize,
            Cursor::SResize => CursorIcon::SResize,
            Cursor::SeResize => CursorIcon::SeResize,
            Cursor::SwResize => CursorIcon::SwResize,
            Cursor::WResize => CursorIcon::WResize,
            Cursor::EwResize => CursorIcon::EwResize,
            Cursor::NsResize => CursorIcon::NsResize,
            Cursor::NeswResize => CursorIcon::NeswResize,
            Cursor::NwseResize => CursorIcon::NwseResize,
            Cursor::ColResize => CursorIcon::ColResize,
            Cursor::RowResize => CursorIcon::RowResize,
            Cursor::AllScroll => CursorIcon::AllScroll,
            Cursor::ZoomIn => CursorIcon::ZoomIn,
            Cursor::ZoomOut => CursorIcon::ZoomOut,
        };
        self.window.set_cursor(icon);
        self.window.set_cursor_visible(true);
    }

    fn request_create_new(&self, parent: WebView, request: CreateNewWebViewRequest) {
        // Until tabs land, a pop-up covers the page that opened it. Closing it
        // (window.close()) brings the opener back.
        let Some(me) = self.me.upgrade() else {
            return;
        };
        let webview = request
            .builder(self.rendering_context.clone())
            .hidpi_scale_factor(self.hidpi_scale())
            .delegate(me)
            .build();
        parent.hide();
        webview.show();
        webview.set_focused(true);
        self.webviews.borrow_mut().push(webview);
        self.window.request_redraw();
    }

    fn notify_closed(&self, webview: WebView) {
        let mut webviews = self.webviews.borrow_mut();
        webviews.retain(|w| w.id() != webview.id());
        if let Some(top) = webviews.last() {
            top.show();
            top.set_focused(true);
        }
        drop(webviews);
        self.window.request_redraw();
    }
}
