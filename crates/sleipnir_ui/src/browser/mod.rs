//! Window-scoped browser panel for macOS and Windows, independent of PTY input.
mod address;
mod automation;
mod input;
mod native;

use crate::chrome::ChromeTokens;
use gpui::{prelude::*, *};
use input::{AddressInput, InputEvent};
use native::{NativeEvent, NativeHost};
use sleipnir_settings::TerminalPalette;
use std::rc::Rc;

pub(crate) enum BrowserEvent {
    Close,
    ReturnToTerminal,
}
impl EventEmitter<BrowserEvent> for BrowserView {}

pub(crate) struct BrowserView {
    address: Entity<AddressInput>,
    focus: FocusHandle,
    host: Option<Rc<NativeHost>>,
    error: Option<String>,
    title: String,
    url: String,
    loading: bool,
    history: (bool, bool),
    pub open: bool,
    blocked: bool,
    agent_access: bool,
    access_epoch: std::sync::Arc<std::sync::atomic::AtomicU64>,
    _events: Task<()>,
    _create: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl BrowserView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let address = cx.new(AddressInput::new);
        let subscription = cx.subscribe_in(
            &address,
            window,
            |this: &mut Self, _, event, window, cx| match event {
                InputEvent::Submit(text) => this.navigate(text, window, cx),
                InputEvent::Escape => {
                    let url = this.url.clone();
                    this.address.update(cx, |input, cx| {
                        input.dirty = false;
                        input.set_url(url, window, cx);
                    });
                    cx.emit(BrowserEvent::ReturnToTerminal);
                }
                InputEvent::FocusPage => {
                    if let Some(host) = &this.host {
                        if this.open && !this.blocked {
                            window.focus(&this.focus, cx);
                            if let Err(error) = host.webview.focus() {
                                this.error = Some(error.to_string());
                            }
                        }
                    }
                }
            },
        );
        let (tx, rx) = async_channel::unbounded();
        // Native callbacks only enqueue data: no GPUI borrow during AppKit/COM callbacks.
        let events = cx.spawn_in(window, async move |this, cx| {
            while let Ok(event) = rx.recv().await {
                if this
                    .update_in(cx, |this, window, cx| {
                        match event {
                            NativeEvent::Loading(loading) => {
                                this.access_epoch
                                    .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                                this.loading = loading;
                                this.error = None;
                            }
                            NativeEvent::Title(title) => this.title = title,
                            NativeEvent::Open(url) => this.navigate(&url, window, cx),
                            NativeEvent::Blocked(message) => this.error = Some(message),
                        }
                        this.poll(window, cx);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let create = cx.spawn_in(window, async move |this, cx| {
            let _ = this.update_in(cx, |this, window, cx| {
                match NativeHost::new(window, tx) {
                    Ok(host) => this.host = Some(host),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        });
        Self {
            address,
            focus: cx.focus_handle(),
            host: None,
            error: None,
            title: "Browser".into(),
            url: "about:blank".into(),
            loading: false,
            history: (false, false),
            open: true,
            blocked: false,
            agent_access: false,
            access_epoch: automation::new_epoch(),
            _events: events,
            _create: create,
            _subscriptions: vec![subscription],
        }
    }
    pub fn focus_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reclaim_focus();
        self.address.update(cx, |input, cx| {
            window.focus(&input.focus, cx);
            input.select_all(cx);
        });
    }
    pub fn reclaim_focus(&self) {
        if let Some(host) = &self.host {
            if host.focused() {
                host.reclaim_focus();
            }
        }
    }
    pub fn set_presentation(&mut self, open: bool, blocked: bool, cx: &mut Context<Self>) {
        if self.open == open && self.blocked == blocked {
            return;
        }
        self.access_epoch
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        if !open {
            self.agent_access = false;
        }
        self.open = open;
        self.blocked = blocked;
        // Restore only after canvas has synchronized current layout, never at old bounds.
        if !open || blocked {
            if let Some(host) = &self.host {
                if let Err(error) = host.set_visible(false) {
                    self.error = Some(error);
                }
            }
        }
        cx.notify();
    }
    pub fn poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(host) = &self.host else {
            return;
        };
        if !self.open || self.blocked {
            return;
        }
        let history = host.history();
        if self.history != history {
            self.history = history;
            cx.notify();
        }
        if let Ok(url) = host.webview.url() {
            if url != self.url && !url.is_empty() {
                self.access_epoch
                    .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                self.url = url;
                cx.notify();
            }
            if self.url != "about:blank" {
                self.address
                    .update(cx, |input, cx| input.set_url(self.url.clone(), window, cx));
            }
        }
        if host.focused() && !self.focus.is_focused(window) {
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }
    fn navigate(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        match address::resolve_address(text) {
            Ok(url) => {
                if let Some(host) = &self.host {
                    match host.webview.load_url(&url) {
                        Ok(()) => {
                            self.error = None;
                            self.loading = true;
                            self.address.update(cx, |input, cx| {
                                input.dirty = false;
                                input.set_url(url, window, cx);
                            });
                        }
                        Err(error) => self.error = Some(error.to_string()),
                    }
                } else {
                    self.error = Some("System WebView is not ready".into());
                }
            }
            Err(error) => self.error = Some(error.into()),
        }
        cx.notify();
    }
    fn navigation(&mut self, back: bool, cx: &mut Context<Self>) {
        if let Some(host) = &self.host {
            if let Err(error) = host.go(back) {
                self.error = Some(error);
            }
        }
        cx.notify();
    }
}

fn button(
    id: &'static str,
    label: &'static str,
    enabled: bool,
    tokens: &ChromeTokens,
) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(28.))
        .min_w(px(26.))
        .px_1()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(13.))
        .text_color(if enabled {
            tokens.fg
        } else {
            tokens.fg_disabled
        })
        .when(enabled, |el| {
            el.cursor_pointer().hover(|el| el.bg(tokens.hover))
        })
        .child(label)
}

impl Render for BrowserView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens =
            ChromeTokens::from_palette(&TerminalPalette::get_global(cx), window.is_window_active());
        let host = self.host.clone();
        let visible = self.open && !self.blocked && self.url != "about:blank";
        let status = if self.blocked {
            "Page hidden while application overlay is open".to_string()
        } else if let Some(error) = &self.error {
            error.clone()
        } else if self.host.is_none() {
            "Starting system browser…".into()
        } else if self.url == "about:blank" {
            "Enter a website or localhost URL above".into()
        } else {
            String::new()
        };
        let back = self.history.0;
        let forward = self.history.1;
        div()
            .id("browser-panel")
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(tokens.surface)
            .track_focus(&self.focus)
            .key_context("BrowserPanel")
            .child(
                div()
                    .h(px(30.))
                    .flex_shrink_0()
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(11.))
                    .text_color(tokens.fg_muted)
                    .child(div().flex_1().min_w_0().truncate().child(if self.loading {
                        format!("Loading · {}", self.title)
                    } else {
                        self.title.clone()
                    }))
                    .child(
                        button(
                            "browser-agent-access",
                            if self.agent_access {
                                "Agent: on"
                            } else {
                                "Agent: off"
                            },
                            true,
                            &tokens,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_agent_access(cx))),
                    )
                    .child(
                        button("browser-terminal", "Terminal", true, &tokens).on_click(
                            cx.listener(|_, _, _, cx| cx.emit(BrowserEvent::ReturnToTerminal)),
                        ),
                    )
                    .child(
                        button("browser-close", "×", true, &tokens)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(BrowserEvent::Close))),
                    ),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .h(px(36.))
                    .px_1()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        button("browser-back", "←", back, &tokens).on_click(cx.listener(
                            move |this, _, _, cx| {
                                if back {
                                    this.navigation(true, cx);
                                }
                            },
                        )),
                    )
                    .child(
                        button("browser-forward", "→", forward, &tokens).on_click(cx.listener(
                            move |this, _, _, cx| {
                                if forward {
                                    this.navigation(false, cx);
                                }
                            },
                        )),
                    )
                    .child(
                        button("browser-reload", "↻", self.host.is_some(), &tokens).on_click(
                            cx.listener(|this, _, _, cx| {
                                if let Some(host) = &this.host {
                                    if let Err(error) = host.webview.reload() {
                                        this.error = Some(error.to_string());
                                    }
                                }
                                cx.notify();
                            }),
                        ),
                    )
                    .child(self.address.clone())
                    .child(
                        button("browser-go", "Go", self.host.is_some(), &tokens).on_click(
                            cx.listener(|this, _, window, cx| {
                                let text = this.address.read(cx).text.clone();
                                this.navigate(&text, window, cx);
                            }),
                        ),
                    ),
            )
            .when(self.error.is_some(), |el| {
                el.child(
                    div()
                        .px_2()
                        .py_1()
                        .text_xs()
                        .text_color(tokens.err)
                        .child(self.error.clone().unwrap_or_default()),
                )
            })
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .bg(tokens.content_bg)
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .p_3()
                            .text_xs()
                            .text_color(tokens.fg_muted)
                            .child(status),
                    )
                    .child(
                        canvas(
                            move |bounds, _, _| {
                                if let Some(host) = host {
                                    let result = host.sync_bounds(bounds).and_then(|_| {
                                        host.set_visible(
                                            visible
                                                && bounds.size.width > px(1.)
                                                && bounds.size.height > px(1.),
                                        )
                                    });
                                    if let Err(error) = result {
                                        log::warn!("browser surface geometry/visibility: {error}");
                                    }
                                }
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0()
                        .size_full(),
                    ),
            )
    }
}

impl Drop for BrowserView {
    fn drop(&mut self) {
        self.access_epoch
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    }
}
