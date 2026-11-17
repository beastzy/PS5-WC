//! Platform font, image, and icon services. None of these APIs open a display.
use std::{
    ffi::{CStr, CString},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr,
};

use cairo::{Context, Format, ImageSurface};
use gio::{
    glib::{self, ffi::gpointer, gobject_ffi::GObject, translate::*},
    prelude::*,
};

#[link(name = "gtk-3")]
extern "C" {
    fn gtk_icon_theme_new() -> *mut GObject;
    fn gtk_icon_theme_set_custom_theme(theme: *mut GObject, name: *const std::ffi::c_char);
    fn gtk_icon_theme_append_search_path(theme: *mut GObject, path: *const std::ffi::c_char);
    fn gtk_icon_theme_lookup_icon(
        theme: *mut GObject,
        name: *const std::ffi::c_char,
        size: i32,
        flags: u32,
    ) -> *mut GObject;
    fn gtk_icon_info_get_filename(info: *mut GObject) -> *const std::ffi::c_char;
}

#[repr(C)]
struct SvgViewport {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[link(name = "rsvg-2")]
extern "C" {
    fn rsvg_handle_new_from_file(
        file: *const std::ffi::c_char,
        error: *mut *mut glib::ffi::GError,
    ) -> *mut GObject;
    fn rsvg_handle_render_document(
        handle: *mut GObject,
        cr: *mut cairo::ffi::cairo_t,
        viewport: *const SvgViewport,
        error: *mut *mut glib::ffi::GError,
    ) -> glib::ffi::gboolean;
}

pub(super) struct IconTheme(glib::Object);

impl IconTheme {
    pub(super) fn new() -> Self {
        // This is an independent theme object, with no GdkScreen or GTK widget.
        let theme: glib::Object = unsafe { from_glib_full(gtk_icon_theme_new()) };
        let source = gio::SettingsSchemaSource::default();
        if let Some(schema) = source.and_then(|source| source.lookup("org.gnome.desktop.interface", true)) {
            let settings = gio::Settings::new_full(&schema, None::<&gio::SettingsBackend>, None);
            let name = settings.string("icon-theme");
            if let Ok(name) = CString::new(name.as_str()) {
                unsafe { gtk_icon_theme_set_custom_theme(theme.as_ptr(), name.as_ptr()) };
            }
        }
        let mut dirs = vec![
            glib::user_data_dir().join("flatpak/exports/share"),
            PathBuf::from("/var/lib/flatpak/exports/share"),
        ];
        dirs.extend(glib::system_data_dirs());
        for dir in dirs {
            if let Ok(path) = CString::new(dir.join("icons").as_os_str().as_bytes()) {
                unsafe { gtk_icon_theme_append_search_path(theme.as_ptr(), path.as_ptr()) };
            }
        }
        Self(theme)
    }

    pub(super) fn path(&self, name: &str) -> Option<PathBuf> {
        let name = CString::new(name).ok()?;
        // GtkIconLookupFlags::FORCE_SIZE
        let info = unsafe { gtk_icon_theme_lookup_icon(self.0.as_ptr(), name.as_ptr(), 181, 1 << 4) };
        if info.is_null() {
            return None;
        }
        let info: glib::Object = unsafe { from_glib_full(info) };
        let filename = unsafe { gtk_icon_info_get_filename(info.as_ptr()) };
        if filename.is_null() {
            return None;
        }
        let filename = unsafe { CStr::from_ptr(filename) };
        Some(PathBuf::from(std::ffi::OsStr::from_bytes(filename.to_bytes())))
    }
}

pub(super) fn load_art(path: &Path) -> Option<ImageSurface> {
    if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("svg") || ext.eq_ignore_ascii_case("svgz"))
    {
        return load_svg(path);
    }
    let decoded = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?
        .into_rgba8();
    let (width, height) = decoded.dimensions();
    let mut surface =
        ImageSurface::create(Format::ARgb32, width.try_into().ok()?, height.try_into().ok()?).ok()?;
    let stride = surface.stride() as usize;
    {
        let mut data = surface.data().ok()?;
        for (target, source) in data.chunks_exact_mut(stride).zip(decoded.rows()) {
            for (dest, pixel) in target.chunks_exact_mut(4).zip(source) {
                let [r, g, b, a] = pixel.0.map(u32::from);
                let premultiply = |component| (component * a + 127) / 255;
                let argb = (a << 24) | (premultiply(r) << 16) | (premultiply(g) << 8) | premultiply(b);
                dest.copy_from_slice(&argb.to_ne_bytes());
            }
        }
    }
    surface.mark_dirty();
    Some(surface)
}

pub(super) fn load_warm_gradient(path: &Path) -> Option<ImageSurface> {
    let mut surface = load_art(path)?;
    let width = surface.width() as usize;
    let stride = surface.stride() as usize;
    // Fade into a more neutral charcoal at the right edge while keeping the
    // brightness and the shape of the original gradient.
    let neutral_mix: Vec<_> = (0..width)
        .map(|x| {
            let normalized = (x as f32 + 0.5) / width as f32;
            let fade = ((normalized - 0.52) / 0.43).clamp(0.0, 1.0);
            0.78 * fade * fade * (3.0 - 2.0 * fade)
        })
        .collect();
    {
        let mut pixels = surface.data().ok()?;
        for row in pixels.chunks_exact_mut(stride) {
            for (pixel, mix) in row.chunks_exact_mut(4).zip(neutral_mix.iter().copied()) {
                let [alpha, red, green, blue] = u32::from_ne_bytes(pixel.try_into().unwrap()).to_be_bytes();
                if mix == 0.0 || blue <= red.max(green) {
                    continue;
                }
                let gray = 0.2126 * f32::from(red) + 0.7152 * f32::from(green) + 0.0722 * f32::from(blue);
                let blend = |channel: u8| f32::from(channel).mul_add(1.0 - mix, gray * mix).round() as u8;
                let argb = u32::from_be_bytes([alpha, blend(red), blend(green), blend(blue)]);
                pixel.copy_from_slice(&argb.to_ne_bytes());
            }
        }
    }
    Some(surface)
}

fn load_svg(path: &Path) -> Option<ImageSurface> {
    const SIZE: i32 = 181;
    let filename = CString::new(path.as_os_str().as_bytes()).ok()?;
    let svg = unsafe { rsvg_handle_new_from_file(filename.as_ptr(), ptr::null_mut()) };
    if svg.is_null() {
        return None;
    }
    let svg: glib::Object = unsafe { from_glib_full(svg) };
    let surface = ImageSurface::create(Format::ARgb32, SIZE, SIZE).ok()?;
    let cr = Context::new(&surface).ok()?;
    let viewport = SvgViewport {
        x: 0.0,
        y: 0.0,
        width: f64::from(SIZE),
        height: f64::from(SIZE),
    };
    let rendered =
        unsafe { rsvg_handle_render_document(svg.as_ptr(), cr.to_raw_none(), &viewport, ptr::null_mut()) };
    (rendered != 0).then_some(surface)
}

#[link(name = "pangocairo-1.0")]
extern "C" {
    fn pango_cairo_create_layout(cr: *mut cairo::ffi::cairo_t) -> *mut GObject;
    fn pango_cairo_show_layout(cr: *mut cairo::ffi::cairo_t, layout: *mut GObject);
}

#[link(name = "pango-1.0")]
extern "C" {
    fn pango_layout_set_text(layout: *mut GObject, text: *const std::ffi::c_char, length: i32);
    fn pango_layout_get_size(layout: *mut GObject, width: *mut i32, height: *mut i32);
    fn pango_layout_set_width(layout: *mut GObject, width: i32);
    fn pango_layout_set_wrap(layout: *mut GObject, wrap: i32);
    fn pango_layout_set_alignment(layout: *mut GObject, alignment: i32);
    fn pango_layout_get_baseline(layout: *mut GObject) -> i32;
    fn pango_layout_set_font_description(layout: *mut GObject, description: gpointer);
    fn pango_font_description_new() -> gpointer;
    fn pango_font_description_free(description: gpointer);
    fn pango_font_description_set_family(description: gpointer, family: *const std::ffi::c_char);
    fn pango_font_description_set_absolute_size(description: gpointer, size: f64);
    fn pango_font_description_set_weight(description: gpointer, weight: i32);
}

#[derive(Clone, Copy)]
pub(super) enum Align {
    Left,
    Center,
    Right,
}

const PANGO_SCALE: f64 = 1024.0;

fn text_layout(cr: &Context, value: &str, size: f64, weight: i32) -> glib::Object {
    let value = value.replace('\0', "");
    // Pango owns the layout and copies the font description and text.
    unsafe {
        let layout: glib::Object = from_glib_full(pango_cairo_create_layout(cr.to_raw_none()));
        let description = pango_font_description_new();
        pango_font_description_set_family(description, c"Inter, Cantarell, sans-serif".as_ptr());
        pango_font_description_set_absolute_size(description, size * PANGO_SCALE);
        pango_font_description_set_weight(description, weight);
        pango_layout_set_font_description(layout.as_ptr(), description);
        pango_font_description_free(description);
        pango_layout_set_text(layout.as_ptr(), value.as_ptr().cast(), value.len() as i32);
        layout
    }
}

pub(super) fn wrapped_label(
    cr: &Context,
    value: &str,
    (x, y): (f64, f64),
    width: f64,
    size: f64,
) -> f64 {
    let layout = text_layout(cr, value, size, 500);
    unsafe {
        pango_layout_set_width(layout.as_ptr(), (width * PANGO_SCALE) as i32);
        pango_layout_set_wrap(layout.as_ptr(), 2); // PANGO_WRAP_WORD_CHAR
        pango_layout_set_alignment(layout.as_ptr(), 1); // PANGO_ALIGN_CENTER
        let mut height = 0;
        pango_layout_get_size(layout.as_ptr(), ptr::null_mut(), &mut height);
        let top = y - f64::from(height) / PANGO_SCALE / 2.0;
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.move_to(x - width / 2.0, top);
        pango_cairo_show_layout(cr.to_raw_none(), layout.as_ptr());
        top
    }
}

pub(super) fn label(
    cr: &Context,
    value: &str,
    (x, baseline): (f64, f64),
    size: f64,
    weight: i32,
    color: (f64, f64, f64),
    align: Align,
) {
    let layout = text_layout(cr, value, size, weight);
    unsafe {
        let mut width = 0;
        pango_layout_get_size(layout.as_ptr(), &mut width, ptr::null_mut());
        let width = f64::from(width) / PANGO_SCALE;
        let left = match align {
            Align::Left => x,
            Align::Center => x - width / 2.0,
            Align::Right => x - width,
        };
        let top = baseline - f64::from(pango_layout_get_baseline(layout.as_ptr())) / PANGO_SCALE;
        cr.set_source_rgb(color.0, color.1, color.2);
        cr.move_to(left, top);
        pango_cairo_show_layout(cr.to_raw_none(), layout.as_ptr());
    }
}
