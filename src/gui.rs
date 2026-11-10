/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Berick's browser chrome: the toolbar with back, forward, reload and the address bar.
//! Drawn with egui on top of the window, with the page painted underneath it.
//! The approach follows Servo's servoshell (ports/servoshell/desktop/gui.rs).

use std::rc::Rc;
use std::sync::Arc;

use egui::text::{CCursor, CCursorRange};
use egui::text_edit::TextEditState;
use egui::{Key, LayerId, Modifiers, PaintCallback, Panel, Vec2};
use egui_glow::{CallbackFn, EguiGlow};
use egui_winit::EventResponse;
use euclid::{Point2D, Rect, Size2D};
use servo::{LoadStatus, OffscreenRenderingContext, RenderingContext, WebView};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

/// Something the user asked the browser to do from the toolbar.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Back,
    Forward,
    Reload,
    /// Load whatever was typed into the address bar.
    Go(String),
}

pub struct Gui {
    rendering_context: Rc<OffscreenRenderingContext>,
    context: EguiGlow,
    /// Height of the toolbar in logical (device independent) pixels.
    toolbar_height: f32,
    location: String,
    /// The user has typed in the address bar and not pressed Enter yet,
    /// so page URL changes must not overwrite what they typed.
    location_dirty: bool,
    focus_location: bool,
    load_status: LoadStatus,
    can_go_back: bool,
    can_go_forward: bool,
    status_text: Option<String>,
    commands: Vec<Command>,
}

impl Drop for Gui {
    fn drop(&mut self) {
        let _ = self.rendering_context.make_current();
        self.context.destroy();
    }
}

impl Gui {
    pub fn new(
        event_loop: &ActiveEventLoop,
        rendering_context: Rc<OffscreenRenderingContext>,
        initial_location: String,
    ) -> Self {
        rendering_context
            .make_current()
            .expect("Could not make RenderingContext current");
        let context = EguiGlow::new(
            event_loop,
            rendering_context.glow_gl_api(),
            None,
            None,
            false,
        );
        context.egui_ctx.options_mut(|options| {
            options.zoom_with_keyboard = false;
            options.fallback_theme = egui::Theme::Light;
        });

        Self {
            rendering_context,
            context,
            toolbar_height: 0.0,
            location: initial_location,
            location_dirty: false,
            focus_location: false,
            load_status: LoadStatus::Complete,
            can_go_back: false,
            can_go_forward: false,
            status_text: None,
            commands: Vec::new(),
        }
    }

    pub fn on_window_event(&mut self, window: &Window, event: &WindowEvent) -> EventResponse {
        self.context.on_window_event(window, event)
    }

    pub fn has_keyboard_focus(&self) -> bool {
        self.context
            .egui_ctx
            .memory(|memory| memory.focused().is_some())
    }

    pub fn surrender_focus(&self) {
        self.context.egui_ctx.memory_mut(|memory| {
            if let Some(focused) = memory.focused() {
                memory.surrender_focus(focused);
            }
        });
    }

    /// Put the cursor in the address bar and select its text (Ctrl+L, Alt+D, F6).
    pub fn focus_location_bar(&mut self) {
        self.focus_location = true;
    }

    /// Toolbar height in logical pixels.
    pub fn toolbar_height(&self) -> f32 {
        self.toolbar_height
    }

    pub fn take_commands(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.commands)
    }

    /// Pull URL, load state and history state from the active WebView.
    /// Returns true if anything changed and the toolbar needs redrawing.
    pub fn update_from_webview(&mut self, webview: Option<&WebView>) -> bool {
        let mut changed = false;

        let load_status = webview.map_or(LoadStatus::Complete, |w| w.load_status());
        if load_status != self.load_status {
            self.load_status = load_status;
            // A new load started or finished: show the real URL again.
            self.location_dirty = false;
            changed = true;
        }

        if !self.location_dirty {
            if let Some(url) = webview.and_then(|w| w.url()) {
                let url = url.to_string();
                if url != self.location {
                    self.location = url;
                    changed = true;
                }
            }
        }

        let (back, forward) =
            webview.map_or((false, false), |w| (w.can_go_back(), w.can_go_forward()));
        if back != self.can_go_back || forward != self.can_go_forward {
            self.can_go_back = back;
            self.can_go_forward = forward;
            changed = true;
        }

        let status_text = webview.and_then(|w| w.status_text());
        if status_text != self.status_text {
            self.status_text = status_text;
            changed = true;
        }

        changed
    }

    fn toolbar_button(text: &str) -> egui::Button<'_> {
        egui::Button::new(text)
            .frame(false)
            .min_size(Vec2 { x: 28.0, y: 28.0 })
    }

    /// Lay out the toolbar, paint the active WebView and resize it to fit the space left over.
    pub fn update(&mut self, window: &Window, webview: Option<&WebView>) {
        self.rendering_context
            .make_current()
            .expect("Could not make RenderingContext current");

        let Self {
            rendering_context,
            context,
            toolbar_height,
            location,
            location_dirty,
            focus_location,
            load_status,
            can_go_back,
            can_go_forward,
            status_text,
            commands,
        } = self;

        context.run(window, |ctx| {
            let frame = egui::Frame::default()
                .fill(ctx.style().visuals.window_fill)
                .inner_margin(6.0);
            let toolbar = Panel::top("toolbar").frame(frame).show_inside(ctx, |ui| {
                ui.allocate_ui_with_layout(
                    ui.available_size(),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        if ui
                            .add_enabled(*can_go_back, Gui::toolbar_button("⏴"))
                            .on_hover_text("Back (Alt+Left)")
                            .clicked()
                        {
                            *location_dirty = false;
                            commands.push(Command::Back);
                        }
                        if ui
                            .add_enabled(*can_go_forward, Gui::toolbar_button("⏵"))
                            .on_hover_text("Forward (Alt+Right)")
                            .clicked()
                        {
                            *location_dirty = false;
                            commands.push(Command::Forward);
                        }

                        let loading =
                            matches!(load_status, LoadStatus::Started | LoadStatus::HeadParsed);
                        let reload_label = if loading { "⌛" } else { "↻" };
                        if ui
                            .add(Gui::toolbar_button(reload_label))
                            .on_hover_text("Reload (Ctrl+R, F5)")
                            .clicked()
                        {
                            *location_dirty = false;
                            commands.push(Command::Reload);
                        }
                        ui.add_space(4.0);

                        let location_id = egui::Id::new("location_input");
                        let location_field = ui.add_sized(
                            ui.available_size(),
                            egui::TextEdit::singleline(location)
                                .id(location_id)
                                .hint_text("Search or enter address"),
                        );
                        if location_field.changed() {
                            *location_dirty = true;
                        }

                        let shortcut = ui.input(|i| {
                            let mut i = i.clone();
                            i.consume_key(Modifiers::COMMAND, Key::L)
                                || (!cfg!(target_os = "macos")
                                    && i.consume_key(Modifiers::ALT, Key::D))
                                || i.consume_key(Modifiers::NONE, Key::F6)
                        });
                        if shortcut || std::mem::take(focus_location) {
                            location_field.request_focus();
                        }
                        // Select the whole address when the bar gains focus.
                        if location_field.gained_focus()
                            && let Some(mut state) = TextEditState::load(ui.ctx(), location_id)
                        {
                            state.cursor.set_char_range(Some(CCursorRange::two(
                                CCursor::new(0),
                                CCursor::new(location.chars().count()),
                            )));
                            state.store(ui.ctx(), location_id);
                        }
                        if location_field.lost_focus() {
                            if ui.input(|i| i.key_pressed(Key::Enter)) {
                                commands.push(Command::Go(location.clone()));
                                *location_dirty = false;
                            } else if ui.input(|i| i.key_pressed(Key::Escape)) {
                                // Esc throws away the edit and shows the page URL again.
                                *location_dirty = false;
                                if let Some(url) = webview.and_then(|w| w.url()) {
                                    *location = url.to_string();
                                }
                            }
                        }
                    },
                );
            });
            *toolbar_height = toolbar.response.rect.max.y;

            // Everything below the toolbar belongs to the page.
            let available_rect = ctx.available_rect_before_wrap();
            let scale = ctx.pixels_per_point();
            if let Some(webview) = webview {
                let wanted = dpi::PhysicalSize::new(
                    (available_rect.width() * scale) as u32,
                    (available_rect.height() * scale) as u32,
                );
                let current = webview.size();
                if current.width as u32 != wanted.width || current.height as u32 != wanted.height {
                    webview.resize(wanted);
                }
                rendering_context
                    .make_current()
                    .expect("Could not make RenderingContext current");
                webview.paint();
                rendering_context.present();
            }

            if let Some(text) = status_text.as_ref() {
                egui::Tooltip::always_open(
                    ctx.clone(),
                    LayerId::new(egui::Order::Tooltip, egui::Id::new("status")),
                    "status".into(),
                    egui::pos2(0.0, available_rect.max.y),
                )
                .show(|ui| ui.add(egui::Label::new(text.clone()).extend()));
            }

            if let Some(render_to_parent) = rendering_context.render_to_parent_callback() {
                ctx.layer_painter(LayerId::background()).add(PaintCallback {
                    rect: available_rect,
                    callback: Arc::new(CallbackFn::new(move |info, painter| {
                        let clip = info.viewport_in_pixels();
                        let rect_in_parent = Rect::new(
                            Point2D::new(clip.left_px, clip.from_bottom_px),
                            Size2D::new(clip.width_px, clip.height_px),
                        );
                        render_to_parent(painter.gl(), rect_in_parent)
                    })),
                });
            }
        });

        if self.context.egui_ctx.has_requested_repaint() {
            window.request_redraw();
        }
    }

    /// Draw the last laid out frame to the window.
    pub fn paint(&mut self, window: &Window) {
        self.rendering_context
            .make_current()
            .expect("Could not make RenderingContext current");
        self.rendering_context
            .parent_context()
            .prepare_for_rendering();
        self.context.paint(window);
        self.rendering_context.parent_context().present();
    }
}
