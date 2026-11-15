//! Keeping the app in the notification area with its window out of sight.
//!
//! A hidden window is not sent paint messages, and the app only runs when it
//! is asked to paint, so hiding the window outright would stop it following
//! the cache. Instead the window is made fully transparent, click-through and
//! absent from the taskbar: invisible to the user, still alive to Windows.
//!
//! Only Windows is supported. Elsewhere [`Tray::new`] returns `None` and
//! closing the window quits as usual.

/// What the user asked for through the tray icon.
#[derive(Clone, Copy, Debug, PartialEq)]
// Nothing produces these where there is no tray.
#[cfg_attr(not(windows), allow(dead_code))]
pub enum TrayEvent {
    Show,
    Quit,
}

#[cfg(windows)]
mod imp {
    use super::TrayEvent;
    use eframe::egui;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::sync::mpsc::{channel, Receiver};
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    #[link(name = "user32")]
    extern "system" {
        fn ShowWindow(hwnd: isize, command: i32) -> i32;
        // 32-bit user32 only has the `Long` names; the `LongPtr` ones are macros for them there.
        #[cfg_attr(target_pointer_width = "32", link_name = "GetWindowLongW")]
        fn GetWindowLongPtrW(hwnd: isize, index: i32) -> isize;
        #[cfg_attr(target_pointer_width = "32", link_name = "SetWindowLongW")]
        fn SetWindowLongPtrW(hwnd: isize, index: i32, value: isize) -> isize;
        fn SetLayeredWindowAttributes(hwnd: isize, key: u32, alpha: u8, flags: u32) -> i32;
        fn SetForegroundWindow(hwnd: isize) -> i32;
        fn IsIconic(hwnd: isize) -> i32;
        fn IsZoomed(hwnd: isize) -> i32;
    }

    const GWL_EXSTYLE: i32 = -20;
    const WS_EX_TRANSPARENT: isize = 0x20;
    const WS_EX_TOOLWINDOW: isize = 0x80;
    const WS_EX_APPWINDOW: isize = 0x4_0000;
    const WS_EX_LAYERED: isize = 0x8_0000;
    const LWA_ALPHA: u32 = 2;
    const SW_HIDE: i32 = 0;
    const SW_SHOWMAXIMIZED: i32 = 3;
    const SW_SHOWNOACTIVATE: i32 = 4;
    const SW_SHOW: i32 = 5;
    const SW_RESTORE: i32 = 9;

    pub struct Tray {
        hwnd: isize,
        // Dropping the icon removes it from the notification area.
        _icon: TrayIcon,
        events: Receiver<TrayEvent>,
        /// While hidden: the window's extended style from before, and whether it was maximised.
        before_hiding: Option<(isize, bool)>,
    }

    impl Tray {
        /// Puts an icon in the notification area. `None` if that is not possible.
        pub fn new(cc: &eframe::CreationContext<'_>, rgba: Vec<u8>, size: u32) -> Option<Self> {
            let RawWindowHandle::Win32(handle) = cc.window_handle().ok()?.as_raw() else { return None };
            let menu = Menu::new();
            let (show, quit) = (MenuItem::new("Show window", true, None), MenuItem::new("Quit", true, None));
            menu.append_items(&[&show, &PredefinedMenuItem::separator(), &quit]).ok()?;
            let icon = TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(false)
                .with_tooltip("Roblox Cache Explorer")
                .with_icon(Icon::from_rgba(rgba, size, size).ok()?)
                .build()
                .ok()?;

            // The handlers run whenever Windows reports a click; each wakes the app to act on it.
            let (tx, events) = channel();
            let (show_id, quit_id) = (show.id().clone(), quit.id().clone());
            let (menu_tx, menu_ctx) = (tx.clone(), cc.egui_ctx.clone());
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                let chosen = if event.id == show_id {
                    TrayEvent::Show
                } else if event.id == quit_id {
                    TrayEvent::Quit
                } else {
                    return;
                };
                let _ = menu_tx.send(chosen);
                menu_ctx.request_repaint();
            }));
            let ctx: egui::Context = cc.egui_ctx.clone();
            TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
                let clicked = matches!(event, TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. })
                    || matches!(event, TrayIconEvent::DoubleClick { button: MouseButton::Left, .. });
                if clicked {
                    let _ = tx.send(TrayEvent::Show);
                    ctx.request_repaint();
                }
            }));
            Some(Self { hwnd: handle.hwnd.get(), _icon: icon, events, before_hiding: None })
        }

        pub fn hidden(&self) -> bool {
            self.before_hiding.is_some()
        }

        /// Takes the window out of sight and off the taskbar while leaving it running.
        pub fn hide(&mut self) {
            if self.hidden() {
                return;
            }
            // SAFETY: `hwnd` is this process's own top-level window, which outlives the tray.
            unsafe {
                let style = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
                let maximised = IsZoomed(self.hwnd) != 0;
                // The taskbar only re-reads a window's style when it is shown again.
                ShowWindow(self.hwnd, SW_HIDE);
                SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, (style | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_TRANSPARENT) & !WS_EX_APPWINDOW);
                SetLayeredWindowAttributes(self.hwnd, 0, 0, LWA_ALPHA);
                // Shown at normal size: a minimised window would not be painted either.
                ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                self.before_hiding = Some((style, maximised));
            }
        }

        /// Brings the window to the front: out of hiding at the size it had, or
        /// simply restored and focused if it was only minimised or covered.
        pub fn show(&mut self) {
            // SAFETY: as in `hide`.
            unsafe {
                match self.before_hiding.take() {
                    Some((style, maximised)) => {
                        ShowWindow(self.hwnd, SW_HIDE);
                        SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, style);
                        ShowWindow(self.hwnd, if maximised { SW_SHOWMAXIMIZED } else { SW_SHOW });
                    }
                    None if IsIconic(self.hwnd) != 0 => drop(ShowWindow(self.hwnd, SW_RESTORE)),
                    None => {}
                }
                SetForegroundWindow(self.hwnd);
            }
        }

        /// The next thing asked for through the icon, if any.
        pub fn poll(&self) -> Option<TrayEvent> {
            self.events.try_recv().ok()
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::TrayEvent;

    /// There is no tray support on this platform, so there are no values of this type.
    pub enum Tray {}

    impl Tray {
        pub fn new(_cc: &eframe::CreationContext<'_>, _rgba: Vec<u8>, _size: u32) -> Option<Self> {
            None
        }

        pub fn hidden(&self) -> bool {
            match *self {}
        }

        pub fn hide(&mut self) {
            match *self {}
        }

        pub fn show(&mut self) {
            match *self {}
        }

        pub fn poll(&self) -> Option<TrayEvent> {
            match *self {}
        }
    }
}

pub use imp::Tray;
