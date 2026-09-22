//! System tray icon and its right-click menu. Falls back to "no tray" (rather than
//! crashing the app) if icon creation fails for any reason, and is a total no-op outside
//! Windows so `app.rs` doesn't need platform `cfg`s of its own.

pub enum TrayEvent {
    ToggleAutostart,
    Quit,
}

#[cfg(windows)]
mod imp {
    use super::TrayEvent;
    use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

    struct Active {
        _icon: TrayIcon,
        autostart_item: CheckMenuItem,
        autostart_id: MenuId,
        quit_id: MenuId,
    }

    pub struct Tray {
        active: Option<Active>,
    }

    impl Tray {
        pub fn new(autostart_enabled: bool) -> Self {
            match Self::try_new(autostart_enabled) {
                Ok(active) => Self {
                    active: Some(active),
                },
                Err(err) => {
                    log::warn!("não foi possível criar o ícone da bandeja: {err}");
                    Self { active: None }
                }
            }
        }

        fn try_new(autostart_enabled: bool) -> Result<Active, String> {
            let menu = Menu::new();
            let autostart_item =
                CheckMenuItem::new("Iniciar com o Windows", true, autostart_enabled, None);
            let quit_item = MenuItem::new("Sair", true, None);
            menu.append(&autostart_item).map_err(|e| e.to_string())?;
            menu.append(&quit_item).map_err(|e| e.to_string())?;

            let autostart_id = autostart_item.id().clone();
            let quit_id = quit_item.id().clone();

            let icon = TrayIconBuilder::new()
                .with_tooltip("win-notch")
                .with_menu(Box::new(menu))
                .with_icon(placeholder_icon()?)
                .build()
                .map_err(|e| e.to_string())?;

            Ok(Active {
                _icon: icon,
                autostart_item,
                autostart_id,
                quit_id,
            })
        }

        pub fn poll_event(&self) -> Option<TrayEvent> {
            let active = self.active.as_ref()?;
            let event = MenuEvent::receiver().try_recv().ok()?;
            if event.id == active.autostart_id {
                Some(TrayEvent::ToggleAutostart)
            } else if event.id == active.quit_id {
                Some(TrayEvent::Quit)
            } else {
                None
            }
        }

        pub fn set_autostart_checked(&self, checked: bool) {
            if let Some(active) = &self.active {
                active.autostart_item.set_checked(checked);
            }
        }
    }

    /// Small solid-color placeholder icon (no bundled .ico asset yet for this MVP).
    fn placeholder_icon() -> Result<Icon, String> {
        let size = 16u32;
        let mut rgba = vec![0u8; (size * size * 4) as usize];
        for pixel in rgba.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[122, 162, 255, 255]);
        }
        Icon::from_rgba(rgba, size, size).map_err(|e| e.to_string())
    }
}

#[cfg(not(windows))]
mod imp {
    use super::TrayEvent;

    pub struct Tray;

    impl Tray {
        pub fn new(_autostart_enabled: bool) -> Self {
            Self
        }

        pub fn poll_event(&self) -> Option<TrayEvent> {
            None
        }

        pub fn set_autostart_checked(&self, _checked: bool) {}
    }
}

pub use imp::Tray;
