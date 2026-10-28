use cocoa::{base::id, foundation::NSString};
use gpui::WindowAppearance;
use objc::{msg_send, sel, sel_impl};
use std::ffi::CStr;

pub(crate) unsafe fn window_appearance_from_native(appearance: id) -> WindowAppearance {
    let name: id = msg_send![appearance, name];
    let name = unsafe { CStr::from_ptr(name.UTF8String()) }.to_string_lossy();
    appearance_from_name(&name).unwrap_or_else(|| {
        println!("unknown appearance: {name:?}");
        WindowAppearance::Light
    })
}

/// Pigeon: by name, including the Increase Contrast appearances, which
/// upstream reported as unknown and turned into Light.
fn appearance_from_name(name: &str) -> Option<WindowAppearance> {
    Some(match name {
        "NSAppearanceNameAqua" | "NSAppearanceNameAccessibilityHighContrastAqua" => {
            WindowAppearance::Light
        }
        "NSAppearanceNameDarkAqua" | "NSAppearanceNameAccessibilityHighContrastDarkAqua" => {
            WindowAppearance::Dark
        }
        "NSAppearanceNameVibrantLight"
        | "NSAppearanceNameAccessibilityHighContrastVibrantLight" => WindowAppearance::VibrantLight,
        "NSAppearanceNameVibrantDark" | "NSAppearanceNameAccessibilityHighContrastVibrantDark" => {
            WindowAppearance::VibrantDark
        }
        _ => return None,
    })
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    pub static NSAppearanceNameAqua: id;
    pub static NSAppearanceNameDarkAqua: id;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn high_contrast_appearances_keep_light_or_dark() {
        let dark = [
            "NSAppearanceNameDarkAqua",
            "NSAppearanceNameAccessibilityHighContrastDarkAqua",
        ];
        for name in dark {
            assert_eq!(appearance_from_name(name), Some(WindowAppearance::Dark));
        }
        assert_eq!(
            appearance_from_name("NSAppearanceNameAccessibilityHighContrastAqua"),
            Some(WindowAppearance::Light)
        );
        assert_eq!(
            appearance_from_name("NSAppearanceNameAccessibilityHighContrastVibrantDark"),
            Some(WindowAppearance::VibrantDark)
        );
        assert_eq!(
            appearance_from_name("NSAppearanceNameAccessibilityHighContrastVibrantLight"),
            Some(WindowAppearance::VibrantLight)
        );
        assert_eq!(appearance_from_name("Something"), None);
    }
}
