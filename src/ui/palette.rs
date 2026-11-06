//! Dynamic Material palette, mirroring QML's `Palette` singleton.
//!
//! The renderer reads the painted palette through the module accessors
//! (`surface()`, `primary()`, …). `PaletteMotion` owns the cover-to-cover
//! crossfade in AppRoot; `publish` makes the current sample available to the
//! retained GPUI element tree.

use std::collections::BTreeMap;
use std::sync::{OnceLock, RwLock};

use crate::ui::anim;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Palette {
    pub background: u32,
    pub on_background: u32,
    pub surface: u32,
    pub surface_dim: u32,
    pub surface_bright: u32,
    pub surface_container_lowest: u32,
    pub surface_container_low: u32,
    pub surface_container: u32,
    pub surface_container_high: u32,
    pub surface_container_highest: u32,
    pub on_surface: u32,
    pub surface_variant: u32,
    pub on_surface_variant: u32,
    pub primary: u32,
    pub on_primary: u32,
    pub primary_container: u32,
    pub on_primary_container: u32,
    pub secondary: u32,
    pub secondary_container: u32,
    pub on_secondary_container: u32,
    pub tertiary: u32,
    pub tertiary_container: u32,
    pub on_tertiary_container: u32,
    pub error: u32,
    pub error_container: u32,
    pub on_error_container: u32,
    pub inverse_surface: u32,
    pub on_inverse_surface: u32,
    pub surface_tint: u32,
    pub outline: u32,
    pub outline_variant: u32,
    pub scrim: u32,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            background: 0x141218,
            on_background: 0xE6E0E9,
            surface: 0x141218,
            surface_dim: 0x141218,
            surface_bright: 0x3B383E,
            surface_container_lowest: 0x0F0D13,
            surface_container_low: 0x1D1B20,
            surface_container: 0x211F26,
            surface_container_high: 0x2B2930,
            surface_container_highest: 0x36343B,
            on_surface: 0xE6E0E9,
            surface_variant: 0x49454F,
            on_surface_variant: 0xCAC4D0,
            primary: 0xD0BCFF,
            on_primary: 0x381E72,
            primary_container: 0x4F378B,
            on_primary_container: 0xEADDFF,
            secondary: 0xCCC2DC,
            secondary_container: 0x4A4458,
            on_secondary_container: 0xE8DEF8,
            tertiary: 0xEFB8C8,
            tertiary_container: 0x633B48,
            on_tertiary_container: 0xFFD8E4,
            error: 0xF2B8B5,
            error_container: 0x8C1D18,
            on_error_container: 0xF9DEDC,
            inverse_surface: 0xE6E0E9,
            on_inverse_surface: 0x322F35,
            surface_tint: 0xD0BCFF,
            outline: 0x938F99,
            outline_variant: 0x49454F,
            scrim: 0x000000,
        }
    }
}

impl Palette {
    pub fn from_roles(roles: &BTreeMap<String, String>) -> Self {
        let mut palette = Self::default();
        macro_rules! role {
            ($key:literal, $field:ident) => {
                if let Some(value) = roles.get($key).and_then(|value| parse_hex(value)) {
                    palette.$field = value;
                }
            };
        }
        role!("background", background);
        role!("on_background", on_background);
        role!("surface", surface);
        role!("surface_dim", surface_dim);
        role!("surface_bright", surface_bright);
        role!("surface_container_lowest", surface_container_lowest);
        role!("surface_container_low", surface_container_low);
        role!("surface_container", surface_container);
        role!("surface_container_high", surface_container_high);
        role!("surface_container_highest", surface_container_highest);
        role!("on_surface", on_surface);
        role!("surface_variant", surface_variant);
        role!("on_surface_variant", on_surface_variant);
        role!("primary", primary);
        role!("on_primary", on_primary);
        role!("primary_container", primary_container);
        role!("on_primary_container", on_primary_container);
        role!("secondary", secondary);
        role!("secondary_container", secondary_container);
        role!("on_secondary_container", on_secondary_container);
        role!("tertiary", tertiary);
        role!("tertiary_container", tertiary_container);
        role!("on_tertiary_container", on_tertiary_container);
        role!("error", error);
        role!("error_container", error_container);
        role!("on_error_container", on_error_container);
        role!("inverse_surface", inverse_surface);
        role!("on_inverse_surface", on_inverse_surface);
        role!("surface_tint", surface_tint);
        role!("outline", outline);
        role!("outline_variant", outline_variant);
        role!("scrim", scrim);
        palette
    }

    pub fn lerp(from: Self, to: Self, t: f32) -> Self {
        macro_rules! mix {
            ($field:ident) => {
                anim::lerp_srgb(from.$field, to.$field, t)
            };
        }
        Self {
            background: mix!(background),
            on_background: mix!(on_background),
            surface: mix!(surface),
            surface_dim: mix!(surface_dim),
            surface_bright: mix!(surface_bright),
            surface_container_lowest: mix!(surface_container_lowest),
            surface_container_low: mix!(surface_container_low),
            surface_container: mix!(surface_container),
            surface_container_high: mix!(surface_container_high),
            surface_container_highest: mix!(surface_container_highest),
            on_surface: mix!(on_surface),
            surface_variant: mix!(surface_variant),
            on_surface_variant: mix!(on_surface_variant),
            primary: mix!(primary),
            on_primary: mix!(on_primary),
            primary_container: mix!(primary_container),
            on_primary_container: mix!(on_primary_container),
            secondary: mix!(secondary),
            secondary_container: mix!(secondary_container),
            on_secondary_container: mix!(on_secondary_container),
            tertiary: mix!(tertiary),
            tertiary_container: mix!(tertiary_container),
            on_tertiary_container: mix!(on_tertiary_container),
            error: mix!(error),
            error_container: mix!(error_container),
            on_error_container: mix!(on_error_container),
            inverse_surface: mix!(inverse_surface),
            on_inverse_surface: mix!(on_inverse_surface),
            surface_tint: mix!(surface_tint),
            outline: mix!(outline),
            outline_variant: mix!(outline_variant),
            scrim: mix!(scrim),
        }
    }
}

fn parse_hex(value: &str) -> Option<u32> {
    let value = value.trim().trim_start_matches('#');
    if value.len() != 6 && value.len() != 8 {
        return None;
    }
    u32::from_str_radix(value, 16).ok()
}

pub struct PaletteMotion {
    from: Palette,
    to: Palette,
    started: f64,
    duration: f32,
    active: bool,
    animated_once: bool,
}

impl PaletteMotion {
    pub fn new(default: Palette) -> Self {
        Self {
            from: default,
            to: default,
            started: 0.0,
            duration: 0.0,
            active: false,
            animated_once: false,
        }
    }

    pub fn set(&mut self, next: Palette, clock: &anim::Clock, animate: bool) {
        let animate = animate && self.animated_once && !anim::REDUCE_MOTION;
        self.animated_once = true;
        if !animate || next == self.to {
            self.from = next;
            self.to = next;
            self.started = clock.now;
            self.duration = 0.0;
            self.active = false;
            return;
        }
        self.from = self.value(clock);
        self.to = next;
        self.started = clock.now;
        self.duration = anim::STANDARD_MS / 1000.0;
        self.active = true;
    }

    pub fn reset(&mut self, clock: &anim::Clock) {
        self.set(Palette::default(), clock, false);
    }

    pub fn value(&self, clock: &anim::Clock) -> Palette {
        if !self.active || self.duration <= 0.0 {
            return self.to;
        }
        let t = ((clock.now - self.started) / self.duration as f64) as f32;
        if t >= 1.0 {
            return self.to;
        }
        Palette::lerp(self.from, self.to, anim::Curve::Standard.sample(t.clamp(0.0, 1.0)))
    }

    pub fn settled(&self, clock: &anim::Clock) -> bool {
        !self.active || clock.now - self.started >= self.duration as f64
    }
}

static CURRENT: OnceLock<RwLock<Palette>> = OnceLock::new();

pub fn publish(palette: Palette) {
    if let Ok(mut current) = CURRENT.get_or_init(|| RwLock::new(Palette::default())).write() {
        *current = palette;
    }
}

pub fn current() -> Palette {
    *CURRENT
        .get_or_init(|| RwLock::new(Palette::default()))
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

macro_rules! roles {
    ($($name:ident),* $(,)?) => {
        $(pub fn $name() -> u32 { current().$name })*
    };
}
roles!(
    background, on_background, surface, surface_dim, surface_bright,
    surface_container_lowest, surface_container_low, surface_container,
    surface_container_high, surface_container_highest, on_surface,
    surface_variant, on_surface_variant, primary, on_primary,
    primary_container, on_primary_container, secondary, secondary_container,
    on_secondary_container, tertiary, tertiary_container, on_tertiary_container,
    error, error_container, on_error_container, inverse_surface,
    on_inverse_surface, surface_tint, outline, outline_variant, scrim,
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_theme_js() {
        let p = Palette::default();
        assert_eq!(p.background, 0x141218);
        assert_eq!(p.surface_container_high, 0x2B2930);
        assert_eq!(p.primary, 0xD0BCFF);
        assert_eq!(p.outline, 0x938F99);
        assert_eq!(p.outline_variant, 0x49454F);
        assert_eq!(p.error_container, 0x8C1D18);
    }

    #[test]
    fn roles_parse_and_fallback() {
        let mut roles = BTreeMap::new();
        roles.insert("primary".to_string(), "#112233".to_string());
        roles.insert("surface_container_high".to_string(), "445566".to_string());
        roles.insert("bogus".to_string(), "#FFFFFF".to_string());
        roles.insert("error".to_string(), "not-a-color".to_string());
        let p = Palette::from_roles(&roles);
        assert_eq!(p.primary, 0x112233);
        assert_eq!(p.surface_container_high, 0x445566);
        assert_eq!(p.error, Palette::default().error);
    }

    #[test]
    fn first_target_snaps_then_fades() {
        let mut clock = anim::Clock::new();
        clock.now = 0.0;
        let mut motion = PaletteMotion::new(Palette::default());
        let first = Palette { primary: 0xFF0000, ..Palette::default() };
        motion.set(first, &clock, true);
        assert_eq!(motion.value(&clock), first);
        clock.now = 1.0;
        let second = Palette { primary: 0x00FF00, ..Palette::default() };
        motion.set(second, &clock, true);
        assert!(!motion.settled(&clock));
        assert_eq!(motion.value(&clock), first);
        clock.now = 1.08;
        let mid = motion.value(&clock).primary;
        assert!(mid != first.primary && mid != second.primary);
        clock.now = 2.0;
        assert_eq!(motion.value(&clock), second);
    }

    #[test]
    fn reset_snaps() {
        let clock = anim::Clock::new();
        let mut motion = PaletteMotion::new(Palette::default());
        motion.set(Palette { primary: 0x123456, ..Palette::default() }, &clock, false);
        motion.reset(&clock);
        assert_eq!(motion.value(&clock), Palette::default());
    }
}
