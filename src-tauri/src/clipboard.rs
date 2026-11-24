use sha2::{Digest, Sha256};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::AppHandle;
use tauri_plugin_clipboard_manager::ClipboardExt;

const SECRET_LIFETIME: Duration = Duration::from_secs(30);

#[derive(Clone, Default)]
pub(crate) struct ClipboardGuard(Arc<Mutex<ClipboardState>>);

#[derive(Default)]
struct ClipboardState {
    generation: u64,
    sensitive: Option<[u8; 32]>,
}

impl ClipboardState {
    fn write<E>(
        &mut self,
        value: &str,
        sensitive: bool,
        write: impl FnOnce(&str) -> Result<(), E>,
    ) -> Result<Option<u64>, E> {
        write(value)?;
        self.generation = self.generation.wrapping_add(1);
        self.sensitive = sensitive.then(|| Sha256::digest(value).into());
        Ok(sensitive.then_some(self.generation))
    }

    fn clear<E>(
        &mut self,
        generation: Option<u64>,
        read: impl FnOnce() -> Result<String, E>,
        clear: impl FnOnce() -> Result<(), E>,
    ) -> Result<(), E> {
        if let Some(expected) = self.sensitive
            && generation.is_none_or(|generation| generation == self.generation)
        {
            let current = zeroize::Zeroizing::new(read()?);
            if <[u8; 32]>::from(Sha256::digest(current.as_bytes())) == expected {
                clear()?;
            }
            self.sensitive = None;
        }
        Ok(())
    }
}

impl ClipboardGuard {
    pub(crate) fn write(
        &self,
        app: &AppHandle,
        value: &str,
        sensitive: bool,
    ) -> Result<(), String> {
        let generation = self
            .0
            .lock()
            .map_err(|_| "Clipboard state unavailable".to_owned())?
            .write(value, sensitive, |value| app.clipboard().write_text(value))
            .map_err(|error| error.to_string())?;
        if let Some(generation) = generation {
            let guard = self.clone();
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(SECRET_LIFETIME).await;
                guard.clear(&app, Some(generation));
            });
        }
        Ok(())
    }

    pub(crate) fn clear_on_lock(&self, app: &AppHandle) {
        self.clear(app, None);
    }

    fn clear(&self, app: &AppHandle, generation: Option<u64>) {
        if let Ok(mut state) = self.0.lock() {
            let _ = state.clear(
                generation,
                || app.clipboard().read_text(),
                || app.clipboard().clear(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Clipboard {
        text: RefCell<String>,
        guard: ClipboardState,
    }

    impl Clipboard {
        fn copy(&mut self, text: &str, sensitive: bool) -> Option<u64> {
            self.guard
                .write(text, sensitive, |value| {
                    self.text.replace(value.to_owned());
                    Ok::<_, ()>(())
                })
                .unwrap()
        }

        fn clear(&mut self, generation: Option<u64>) {
            self.guard
                .clear(
                    generation,
                    || Ok(self.text.borrow().clone()),
                    || {
                        self.text.replace(String::new());
                        Ok::<_, ()>(())
                    },
                )
                .unwrap();
        }
    }

    #[test]
    fn old_timer_preserves_same_text_from_a_new_copy() {
        let mut clipboard = Clipboard::default();
        let first = clipboard.copy("fixture-secret", true);
        let second = clipboard.copy("fixture-secret", true);
        clipboard.clear(first);
        assert_eq!(&*clipboard.text.borrow(), "fixture-secret");
        clipboard.clear(second);
        assert!(clipboard.text.borrow().is_empty());
    }

    #[test]
    fn locking_preserves_text_replaced_by_another_app() {
        let mut clipboard = Clipboard::default();
        clipboard.copy("fixture-secret", true);
        clipboard.text.replace("my notes".to_owned());
        clipboard.clear(None);
        assert_eq!(&*clipboard.text.borrow(), "my notes");
        clipboard.copy("fixture-secret", true);
        clipboard.clear(None);
        assert!(clipboard.text.borrow().is_empty());
    }

    #[test]
    fn successful_public_copy_disarms_cleanup_but_failed_copy_does_not() {
        let mut clipboard = Clipboard::default();
        let first = clipboard.copy("fixture-secret", true);
        clipboard
            .guard
            .write("notes", false, |_| Err(()))
            .unwrap_err();
        clipboard.clear(first);
        assert!(clipboard.text.borrow().is_empty());

        let second = clipboard.copy("fixture-secret", true);
        clipboard.copy("fixture-secret", false);
        clipboard.clear(second);
        clipboard.clear(None);
        assert_eq!(&*clipboard.text.borrow(), "fixture-secret");
    }
}
