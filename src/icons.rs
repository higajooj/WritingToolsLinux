//! Bundled icons. The action icons are drawn in `currentColor`, which is
//! replaced with the theme's foreground so one asset serves light and dark.

use std::cell::RefCell;
use std::collections::HashMap;

use adw::prelude::*;
use gtk::{gdk, gdk_pixbuf::Pixbuf, gio, glib};

pub const APP_ICON_PNG: &[u8] = include_bytes!("../assets/icons/app_icon.png");

fn source(name: &str) -> Option<&'static [u8]> {
    macro_rules! icons {
        ($($name:literal),* $(,)?) => {
            match name {
                $($name => Some(include_bytes!(concat!("../assets/icons/", $name, ".svg")).as_slice()),)*
                _ => None,
            }
        };
    }
    icons!(
        "briefcase",
        "check",
        "clipboard_ready",
        "concise",
        "copy",
        "cross",
        "custom",
        "keypoints",
        "list",
        "magnifying-glass",
        "minus",
        "pencil",
        "plus",
        "provider_gemini",
        "provider_ollama",
        "provider_openai",
        "regenerate",
        "reset",
        "restore",
        "rewrite",
        "send",
        "smiley-face",
        "summary",
        "table",
    )
}

/// Normalise an options.json icon reference such as `icons/summary`.
pub fn option_icon_name(icon: &str) -> &str {
    let name = icon.rsplit('/').next().unwrap_or(icon);
    let name = name.strip_suffix(".svg").unwrap_or(name);
    if source(name).is_some() { name } else { "custom" }
}

/// Rasterise an icon at `size` pixels, recoloured when `color` is given.
pub fn pixbuf(name: &str, size: i32, color: Option<&str>) -> Option<Pixbuf> {
    let svg = source(name)?;
    let bytes = match color {
        Some(color) => {
            glib::Bytes::from_owned(String::from_utf8_lossy(svg).replace("currentColor", color).into_bytes())
        }
        None => glib::Bytes::from_static(svg),
    };
    let stream = gio::MemoryInputStream::from_bytes(&bytes);
    Pixbuf::from_stream_at_scale(&stream, size, size, true, gio::Cancellable::NONE)
        .inspect_err(|e| log::warn!("Could not render icon {name}: {e}"))
        .ok()
}

fn foreground() -> &'static str {
    if adw::StyleManager::default().is_dark() { "#ffffff" } else { "#333333" }
}

thread_local! {
    static CACHE: RefCell<HashMap<(String, i32, &'static str), gdk::Texture>> = RefCell::new(HashMap::new());
}

/// An icon in the current theme's foreground colour. Rendered at twice the
/// display size so it stays sharp on HiDPI screens.
#[allow(deprecated)] // Texture::for_pixbuf: still the simplest bridge from a Pixbuf.
pub fn themed_texture(name: &str, size: i32) -> Option<gdk::Texture> {
    // Multi-colour marks (Gemini's gradient) carry no currentColor, so the
    // substitution leaves them untouched.
    let color = foreground();
    let key = (name.to_owned(), size, color);
    if let Some(texture) = CACHE.with(|cache| cache.borrow().get(&key).cloned()) {
        return Some(texture);
    }
    let texture = gdk::Texture::for_pixbuf(&pixbuf(name, size * 2, Some(color))?);
    CACHE.with(|cache| cache.borrow_mut().insert(key, texture.clone()));
    Some(texture)
}

/// An image that follows light/dark changes.
pub fn image(name: &str, size: i32) -> gtk::Image {
    let image = gtk::Image::new();
    image.set_pixel_size(size);
    image.set_paintable(themed_texture(name, size).as_ref());
    let name = name.to_owned();
    let weak = image.downgrade();
    let handler = std::cell::Cell::new(None);
    handler.set(Some(adw::StyleManager::default().connect_dark_notify(move |_| {
        if let Some(image) = weak.upgrade() {
            image.set_paintable(themed_texture(&name, size).as_ref());
        }
    })));
    image.connect_destroy(move |_| {
        if let Some(handler) = handler.take() {
            adw::StyleManager::default().disconnect(handler);
        }
    });
    image
}

/// An icon-only button with a tooltip.
pub fn button(name: &str, size: i32, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.set_child(Some(&image(name, size)));
    button.set_tooltip_text(Some(tooltip));
    button.update_property(&[gtk::accessible::Property::Label(tooltip)]);
    button
}

/// ARGB32 (network byte order) pixels for the tray.
pub fn argb(pixbuf: &Pixbuf) -> (i32, i32, Vec<u8>) {
    let pixbuf = if pixbuf.has_alpha() {
        pixbuf.clone()
    } else {
        pixbuf.add_alpha(false, 0, 0, 0).unwrap_or_else(|_| pixbuf.clone())
    };
    let (width, height, stride) = (pixbuf.width(), pixbuf.height(), pixbuf.rowstride() as usize);
    let bytes = pixbuf.read_pixel_bytes();
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height as usize {
        for px in bytes[row * stride..].as_chunks::<4>().0.iter().take(width as usize) {
            data.extend_from_slice(&[px[3], px[0], px[1], px[2]]);
        }
    }
    (width, height, data)
}

pub fn app_icon_pixbuf(size: i32) -> Option<Pixbuf> {
    let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_static(APP_ICON_PNG));
    Pixbuf::from_stream_at_scale(&stream, size, size, true, gio::Cancellable::NONE).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_icons_resolve() {
        assert_eq!(option_icon_name("icons/magnifying-glass"), "magnifying-glass");
        assert_eq!(option_icon_name("summary"), "summary");
        assert_eq!(option_icon_name("icons/nope"), "custom");
        for (_, entry) in crate::options::defaults() {
            assert!(source(option_icon_name(&entry.icon)).is_some(), "{}", entry.icon);
        }
    }

    #[test]
    fn icons_render_recoloured() {
        let pixbuf = pixbuf("copy", 32, Some("#ff0000")).unwrap();
        assert_eq!((pixbuf.width(), pixbuf.height()), (32, 32));
        let (_, _, argb) = argb(&pixbuf);
        // Some opaque pixel is pure red.
        assert!(argb.as_chunks::<4>().0.contains(&[255, 255, 0, 0]));
        assert!(app_icon_pixbuf(64).is_some());
    }
}
