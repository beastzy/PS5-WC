//! Persistent renderer textures for the regions of Home that change independently.
use std::{collections::{HashMap, VecDeque}, time::Instant};

use cairo::{Context, Format, ImageSurface, Operator};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                utils::CropRenderElement,
                Kind, Wrap,
            },
            ImportAll, ImportMem, Renderer,
        },
    },
    utils::{Logical, Rectangle, Size, Transform},
};

use crate::render::CustomRenderElements;

use super::{
    library::Grid,
    visual::{self, Layout, Rect, DESIGN_WIDTH, SELECTED, TILE},
    Home, Overlay, Page,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Revision {
    state: u64,
    frame: Option<Instant>,
    detail: u64,
}

impl Revision {
    fn fixed(state: u64, detail: u64) -> Self {
        Self {
            state,
            frame: None,
            detail,
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct Scene {
    background: Option<Layer>,
    welcome_word: Option<Layer>,
    welcome_word_bounds: (f64, f64),
    welcome_particles: Vec<Option<Layer>>,
    header: Option<Layer>,
    carousel_focus: Option<Layer>,
    carousel_tiles: [Vec<Option<Layer>>; 2],
    growing_tiles: [Vec<Option<Layer>>; 2],
    details_cache: VecDeque<(DetailKey, Option<Layer>)>,
    hero_cache: VecDeque<(usize, Option<Layer>)>,
    details_position: Option<(Layer, Details)>,
    previous_details: Option<(Layer, Details)> ,
    notices: Option<Layer>,
    control: Option<Layer>,
    control_panel: Option<Layer>,
    control_scrim: Option<Layer>,
    launch_cover: Option<Layer>,
    panels: Option<Layer>,
    library_tiles: HashMap<usize, Layer>,
    library_heading: Option<Layer>,
}

#[derive(Clone, Copy, Debug)]
struct Details {
    page: Page,
    index: usize,
    region: Region,
    opacity: f64,
    offset: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DetailKey {
    page: Page,
    index: usize,
    focus: usize,
    running: bool,
    contrast: bool,
    logo: bool,
}

#[derive(Clone, Debug)]
struct Layer {
    raster: (i32, i32),
    opaque: bool,
    revision: Option<Revision>,
    buffer: MemoryRenderBuffer,
}

impl Layer {
    fn new(raster: (i32, i32), opaque: bool) -> Self {
        let buffer = MemoryRenderBuffer::new(
            Fourcc::Argb8888,
            raster,
            1,
            Transform::Normal,
            opaque.then(|| vec![Rectangle::from_size(raster.into())]),
        );
        Self {
            raster,
            opaque,
            revision: None,
            buffer,
        }
    }

    fn upload(
        &mut self,
        rev: Revision,
        paint: impl FnOnce(&Context) -> Result<(), cairo::Error>,
    ) -> Result<(), cairo::Error> {
        if self.revision == Some(rev) {
            return Ok(());
        }
        let (width, height) = self.raster;
        let stride = Format::ARgb32.stride_for_width(width as u32)?;
        self.buffer.render().draw(|pixels| {
            debug_assert_eq!(pixels.len(), stride as usize * height as usize);
            // Cairo and its context are dropped before the draw closure releases
            // the mutable buffer. Neither is returned or stored by the painter.
            let surface = unsafe {
                ImageSurface::create_for_data_unsafe(
                    pixels.as_mut_ptr(),
                    Format::ARgb32,
                    width,
                    height,
                    stride,
                )?
            };
            let cr = Context::new(&surface)?;
            if !self.opaque {
                cr.set_operator(Operator::Clear);
                cr.paint()?;
                cr.set_operator(Operator::Over);
            }
            paint(&cr)?;
            surface.flush();
            Ok::<_, cairo::Error>(vec![Rectangle::from_size(self.raster.into())])
        })?;
        self.revision = Some(rev);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
struct Region {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

fn raster_size(region: Region, divisor: i32) -> (i32, i32) {
    (
        (region.width + divisor - 1) / divisor,
        (region.height + divisor - 1) / divisor,
    )
}

#[derive(Clone, Copy)]
struct Output {
    width: i32,
    height: i32,
    scale: f64,
}

impl Output {
    // One viewport pixel need not correspond to a fresh CPU-rasterized
    // sample when the artwork itself has already reached its source resolution.
    fn backdrop_divisor(self, art: Option<&ImageSurface>) -> i32 {
        art.map_or(1, |art| {
            let upscale = (f64::from(self.width) / f64::from(art.width()))
                .max(f64::from(self.height) / f64::from(art.height()));
            (upscale / 1.4).floor().max(1.0) as i32
        })
    }

    fn band(self, y: f64, bottom: f64) -> Region {
        self.region(0.0, y, DESIGN_WIDTH, bottom)
    }

    fn region(self, x: f64, y: f64, right: f64, bottom: f64) -> Region {
        let factor = f64::from(self.width) / DESIGN_WIDTH;
        let left = (x * factor).floor().clamp(0.0, f64::from(self.width - 1)) as i32;
        let top = (y * factor).floor().clamp(0.0, f64::from(self.height - 1)) as i32;
        let right = (right * factor)
            .ceil()
            .clamp(f64::from(left + 1), f64::from(self.width)) as i32;
        let bottom = (bottom * factor)
            .ceil()
            .clamp(f64::from(top + 1), f64::from(self.height)) as i32;
        Region {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        }
    }

    fn add<R>(
        self,
        elements: &mut Vec<CustomRenderElements<R>>,
        renderer: &mut R,
        slot: &mut Option<Layer>,
        region: Region,
        divisor: i32,
        opaque: bool,
        rev: Revision,
        alpha: f64,
        offset: f64,
        paint: impl FnOnce(&Context) -> Result<(), cairo::Error>,
    ) where
        R: Renderer + ImportAll + ImportMem,
        R::TextureId: Clone + Send + 'static,
    {
        if alpha <= 0.0 {
            return;
        }
        let raster = raster_size(region, divisor);
        let layer = match slot {
            Some(layer) if layer.raster == raster && layer.opaque == opaque => layer,
            _ => slot.insert(Layer::new(raster, opaque)),
        };
        if let Err(error) = layer.upload(rev, |cr| {
            let factor = f64::from(self.width) / DESIGN_WIDTH;
            cr.scale(
                f64::from(raster.0) / f64::from(region.width),
                f64::from(raster.1) / f64::from(region.height),
            );
            cr.translate(-f64::from(region.x), -f64::from(region.y));
            cr.scale(factor, factor);
            paint(cr)
        }) {
            tracing::warn!(%error, "Could not paint Home texture");
            return;
        }
        let at_y = f64::from(region.y) + offset * f64::from(self.width) / DESIGN_WIDTH;
        let fills_output = opaque && region.width == self.width && region.height == self.height;
        self.add_element(
            elements,
            renderer,
            layer,
            (f64::from(region.x), at_y),
            (f64::from(region.width), f64::from(region.height)),
            alpha,
            fills_output,
            None,
        );
    }

    fn add_element<R>(
        self,
        elements: &mut Vec<CustomRenderElements<R>>,
        renderer: &mut R,
        layer: &Layer,
        at: (f64, f64),
        extent: (f64, f64),
        alpha: f64,
        fills_output: bool,
        source: Option<Rectangle<f64, Logical>>,
    ) where
        R: Renderer + ImportAll + ImportMem,
        R::TextureId: Clone + Send + 'static,
    {
        let logical = |pixels: f64| {
            let size = pixels / self.scale;
            // Round opaque viewport coverage up so fractional scale can't leave a bare edge.
            if fills_output {
                size.ceil() as i32
            } else {
                (size.round() as i32).max(1)
            }
        };
        let size = Size::<i32, Logical>::from((logical(extent.0), logical(extent.1)));
        let (w, h) = layer.raster;
        let element = MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            at,
            &layer.buffer,
            Some(alpha as f32),
            source.or_else(|| Some(Rectangle::from_size((f64::from(w), f64::from(h)).into()))),
            Some(size),
            Kind::Unspecified,
        );
        match element {
            Ok(element) => elements.push(CustomRenderElements::Home(element)),
            Err(error) => tracing::warn!(?error, "Could not upload Home texture"),
        }
    }
}

fn lru<K: PartialEq>(
    cache: &mut VecDeque<(K, Option<Layer>)>,
    key: K,
    capacity: usize,
) -> &mut Option<Layer> {
    let layer = cache.iter().position(|(candidate, _)| *candidate == key)
        .and_then(|index| cache.remove(index))
        .and_then(|(_, layer)| layer);
    cache.push_front((key, layer));
    cache.truncate(capacity);
    &mut cache.front_mut().unwrap().1
}

impl Scene {
    pub(super) fn invalidate_games(&mut self) {
        self.carousel_tiles[0].clear();
        self.growing_tiles[0].clear();
        self.details_cache.retain(|(key, _)| key.page != Page::Games);
        self.hero_cache.clear();
        self.details_position = None;
        self.previous_details = None;
    }

    pub(super) fn invalidate_apps(&mut self) {
        self.carousel_tiles[1].clear();
        self.growing_tiles[1].clear();
        self.details_cache.retain(|(key, _)| key.page != Page::Apps);
        self.details_position = None;
        self.previous_details = None;
    }

    pub(super) fn render<R>(
        &mut self,
        home: &mut Home,
        renderer: &mut R,
        size: (i32, i32),
        scale: f64,
        now: Instant,
        time: &str,
        minute: u64,
    ) -> Vec<CustomRenderElements<R>>
    where
        R: Renderer + ImportAll + ImportMem,
        R::TextureId: Clone + Send + 'static,
    {
        let output = Output {
            width: size.0,
            height: size.1,
            scale,
        };
        let layout = Layout {
            height: DESIGN_WIDTH * f64::from(size.1) / f64::from(size.0),
        };
        let whole = output.band(0.0, layout.height);
        let mut elements = Vec::with_capacity(5);
        let intro = home.intro_at(now);
        let version = home.version;

        if let Some((serial, width)) = home.launch_cover_clip(now, size.0) {
            let mut cover = Vec::with_capacity(1);
            let art = home.launch_art();
            output.add(
                &mut cover,
                renderer,
                &mut self.launch_cover,
                whole,
                output.backdrop_divisor(art.as_ref()),
                true,
                Revision::fixed(serial, u64::from(art.is_some())),
                1.0,
                0.0,
                |cr| visual::paint_launch_cover(cr, layout, art.as_ref()),
            );
            if let Some(CustomRenderElements::Home(element)) = cover.pop() {
                let crop = Rectangle::from_size((width, size.1).into());
                if let Some(element) = CropRenderElement::from_element(element, scale, crop) {
                    elements.push(CustomRenderElements::Cover(element));
                }
            }
            if !home.visible {
                return elements;
            }
        } else {
            self.launch_cover = None;
        }

        if let Some((alpha, id)) = home.status.opacity(now).filter(|_| !home.do_not_disturb) {
            let alpha = if home.reduced_motion { 1.0 } else { alpha };
            let bounds = output.region(2585.0, layout.y(207.0) - 2.0, 2840.0, layout.y(220.0) + 265.0);
            output.add(
                &mut elements,
                renderer,
                &mut self.notices,
                bounds,
                1,
                false,
                Revision::fixed(id, home.status.revision),
                alpha,
                0.0,
                |cr| home.paint_peripherals(cr, layout),
            );
        }
        let control_open = home.overlay == Overlay::Control;
        if control_open {
            let progress = home.control_progress(now);
            if home.has_control_panel() {
                let panel = output.region(480.0, home.panel_top(layout), 2340.0, layout.y(1577.0));
                output.add(
                    &mut elements,
                    renderer,
                    &mut self.control_panel,
                    panel,
                    1,
                    false,
                    Revision::fixed(version, 0),
                    progress,
                    layout.y(34.0) * (1.0 - progress),
                    |cr| home.paint_control_panel(cr, layout),
                );
            }
            let dock = output.band(layout.control_top(home.control_power.is_some()), layout.height);
            output.add(
                &mut elements,
                renderer,
                &mut self.control,
                dock,
                1,
                false,
                Revision::fixed(version, 0),
                progress,
                layout.y(22.0) * (1.0 - progress),
                |cr| home.paint_control(cr, layout),
            );
            output.add(
                &mut elements,
                renderer,
                &mut self.control_scrim,
                whole,
                size.0.max(size.1),
                false,
                Revision::fixed(0, 0),
                1.0,
                0.0,
                |cr| home.paint_control_scrim(cr, layout),
            );
            if !home.visible {
                return elements;
            }
        }

        if let Some(elapsed) = intro.filter(|_| !control_open).and_then(|phase| phase.welcome) {
            self.render_welcome(home, renderer, &mut elements, output, layout, whole, elapsed);
            return elements;
        }

        if home.library.open && matches!(home.overlay, Overlay::None | Overlay::Control)
            && home.notice.is_none()
        {
            self.render_library(home, renderer, &mut elements, output, layout, whole, now, time, minute);
            return elements;
        }
        if !home.library.open {
            self.library_tiles.clear();
            self.library_heading = None;
        }

        if !home.visible
            || (!control_open && home.overlay != Overlay::None)
            || home.page == Page::Settings
            || home.notice.is_some()
        {
            output.add(
                &mut elements,
                renderer,
                &mut self.panels,
                whole,
                1,
                home.visible,
                Revision::fixed(version, minute),
                1.0,
                0.0,
                |cr| home.paint_full(cr, layout, time, now),
            );
            return elements;
        }

        self.panels = None;
        self.welcome_word = None;
        self.welcome_particles.clear();
        let header = intro.map_or(1.0, |phase| phase.header);
        let info = intro.map_or(1.0, |phase| phase.info);
        let entrance = intro.and_then(|phase| phase.entrance);
        let moving = entrance.is_some()
            || home
                .carousel_motion
                .as_ref()
                .is_some_and(|motion| motion.active(now));
        output.add(
            &mut elements,
            renderer,
            &mut self.header,
            output.band(0.0, layout.y(181.0)),
            1,
            false,
            Revision::fixed(home.header_revision(), minute),
            header,
            0.0,
            |cr| home.paint_header(cr, layout, time),
        );
        let index = home.selected();
        let progress = home.carousel_motion.as_ref().map_or(1.0, |motion| motion.detail_progress(now));
        let old = self.details_position.take();
        if let Some((layer, details)) = old {
            if (details.page, details.index) == (home.page, index) {
                self.details_position = Some((layer, details));
            } else if progress < 1.0 && details.page == home.page {
                // When input retargets a transition, keep whichever details
                // texture contributed the most to the last displayed frame.
                if self.previous_details.as_ref().is_none_or(|(_, previous)| details.opacity >= previous.opacity) {
                    self.previous_details = Some((layer, details));
                }
            }
        }
        if progress >= 1.0 || self.previous_details.as_ref().is_some_and(|(_, details)| details.page != home.page) {
            self.previous_details = None;
        }
        if let Some((left, top, right, logo)) = home.info_bounds(layout) {
            let key = DetailKey {
                page: home.page,
                index,
                focus: home.action_focus,
                running: home.entries().get(index).is_some_and(|entry| home.running_games.contains(&entry.id)),
                contrast: home.high_contrast,
                logo,
            };
            let slot = lru(&mut self.details_cache, key, 4);
            let bounds = output.region(left, top, right, layout.y(1690.0));
            let opacity = info * progress;
            let offset = layout.y(27.0) * (1.0 - opacity);
            output.add(
                &mut elements,
                renderer,
                slot,
                bounds,
                1,
                false,
                Revision::fixed(0, 0),
                opacity,
                offset,
                |cr| home.paint_info(cr, layout),
            );
            if let Some(layer) = slot.as_ref().filter(|_| opacity > 0.0) {
                self.details_position = Some((layer.clone(), Details { page: home.page, index, region: bounds, opacity, offset }));
            }
        }
        if let Some((layer, details)) = &self.previous_details {
            let region = details.region;
            let y = f64::from(region.y) + (details.offset - progress * layout.y(14.0)) * f64::from(output.width) / DESIGN_WIDTH;
            output.add_element(
                &mut elements, renderer, layer, (f64::from(region.x), y),
                (f64::from(region.width), f64::from(region.height)),
                details.opacity * (1.0 - progress), false, None,
            );
        }
        self.render_carousel(
            home,
            renderer,
            &mut elements,
            output,
            layout,
            now,
            entrance,
            moving,
        );
        let (background, hero) = home.background_art();
        let divisor = output.backdrop_divisor(background.as_ref());
        let slot = if hero {
            // Keep neighboring backdrops resident during rapid navigation,
            // with a smaller cache for full-resolution or 4K artwork.
            let capacity = if raster_size(whole, divisor).0 * raster_size(whole, divisor).1 > 3_000_000 { 2 } else { 3 };
            lru(&mut self.hero_cache, home.game, capacity)
        } else {
            &mut self.background
        };
        output.add(
            &mut elements,
            renderer,
            slot,
            whole,
            divisor,
            true,
            Revision::fixed(1, u64::from(background.is_some())),
            1.0,
            0.0,
            |cr| home.paint_background_art(cr, layout, background.as_ref(), hero),
        );
        elements
    }

    fn render_welcome<R>(
        &mut self,
        home: &mut Home,
        renderer: &mut R,
        elements: &mut Vec<CustomRenderElements<R>>,
        output: Output,
        layout: Layout,
        whole: Region,
        elapsed: f64,
    ) where
        R: Renderer + ImportAll + ImportMem,
        R::TextureId: Clone + Send + 'static,
    {
        let factor = f64::from(output.width) / DESIGN_WIDTH;
        self.welcome_particles
            .resize_with(home.particles.len(), Default::default);
        let particle_time = home.intro.particle_time(elapsed);
        for (index, particle, x, y, alpha) in
            visual::visible_particles(layout, particle_time, &home.particles)
        {
            let alpha = alpha * home.intro.particle_opacity(index, elapsed);
            if alpha <= 0.0 {
                continue;
            }
            let x = x * factor;
            let y = y * factor;
            let radius = particle.extent() * factor + 1.0;
            if x + radius < 0.0
                || y + radius < 0.0
                || x - radius >= f64::from(output.width)
                || y - radius >= f64::from(output.height)
            {
                continue;
            }
            let logical = (radius * 2.0 / output.scale).ceil().max(1.0) as i32;
            let physical = f64::from(logical) * output.scale;
            let side = physical.ceil() as i32 + 4;
            let center = f64::from(side) / 2.0;
            let layer = match &mut self.welcome_particles[index] {
                Some(layer) if layer.raster == (side, side) => layer,
                slot => slot.insert(Layer::new((side, side), false)),
            };
            if let Err(error) = layer.upload(Revision::fixed(0, 0), |cr| {
                cr.translate(center, center);
                cr.scale(factor, factor);
                particle.paint(cr, 0.0, 0.0, 1.0)
            }) {
                tracing::warn!(%error, "Could not paint Welcome particle");
                continue;
            }
            let left = (x - physical / 2.0).round();
            let top = (y - physical / 2.0).round();
            // Move sampling within the padded sprite so tiny particles retain
            // subpixel movement on every output scale.
            let from = Rectangle::new(
                (center + left - x, center + top - y).into(),
                (physical.round(), physical.round()).into(),
            );
            match MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                (left, top),
                &layer.buffer,
                Some(alpha as f32),
                Some(from),
                Some((logical, logical).into()),
                Kind::Unspecified,
            ) {
                Ok(element) => elements.push(CustomRenderElements::Home(element)),
                Err(error) => tracing::warn!(?error, "Could not upload Welcome particle"),
            }
        }
        let reveal = visual::welcome_reveal(elapsed);
        if reveal > 0.0 {
            let count = elements.len();
            let bounds = &mut self.welcome_word_bounds;
            let band = output.band(layout.height * 0.42, layout.height * 0.58);
            output.add(
                elements,
                renderer,
                &mut self.welcome_word,
                band,
                1,
                false,
                Revision::fixed(0, 0),
                1.0,
                0.0,
                |cr| {
                    *bounds = visual::paint_welcome_word(cr, layout, 5.0)?;
                    Ok(())
                },
            );
            if elements.len() > count {
                if let Some(CustomRenderElements::Home(word)) = elements.pop() {
                    let edge = (bounds.0 + bounds.1 * reveal) * factor;
                    let right = edge.floor() as i32;
                    let crop = Rectangle::from_size((right, output.height).into());
                    if let Some(word) = CropRenderElement::from_element(Wrap::from(word), output.scale, crop)
                    {
                        elements.push(CustomRenderElements::HomeClip(word));
                    }
                    // Blend the last column so the reveal moves smoothly even
                    // when the words advance by less than one pixel per refresh.
                    let fraction = edge.fract();
                    if reveal < 1.0 && fraction > 0.0 {
                        if let Some(layer) = &self.welcome_word {
                            let count = elements.len();
                            output.add_element(
                                elements,
                                renderer,
                                layer,
                                (f64::from(band.x), f64::from(band.y)),
                                (f64::from(band.width), f64::from(band.height)),
                                fraction,
                                false,
                                None,
                            );
                            if elements.len() > count {
                                if let Some(CustomRenderElements::Home(word)) = elements.pop() {
                                    let strip = Rectangle::new((right, 0).into(), (1, output.height).into());
                                    if let Some(word) =
                                        CropRenderElement::from_element(Wrap::from(word), output.scale, strip)
                                    {
                                        elements.push(CustomRenderElements::HomeClip(word));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        output.add(
            elements,
            renderer,
            &mut self.background,
            whole,
            1,
            true,
            Revision::fixed(0, 0),
            1.0,
            0.0,
            |cr| home.paint_warm_gradient(cr, layout),
        );
    }

    fn render_library<R>(
        &mut self,
        home: &mut Home,
        renderer: &mut R,
        elements: &mut Vec<CustomRenderElements<R>>,
        output: Output,
        layout: Layout,
        whole: Region,
        now: Instant,
        time: &str,
        minute: u64,
    ) where
        R: Renderer + ImportAll + ImportMem,
        R::TextureId: Clone + Send + 'static,
    {
        let grid = Grid::new(layout, home.page);
        let count = home.library_entries().len();
        let offset = home.library.offset;
        let version = home.version;
        let library_revision = home.library.revision;
        let games = home.page == Page::Games;
        let factor = f64::from(output.width) / DESIGN_WIDTH;
        let side = (grid.size() * factor).round().max(1.0) as i32;
        let crop_top = (grid.top * factor).round() as i32;
        let viewport = Rectangle::new((0, crop_top).into(), (output.width, output.height - crop_top).into());

        output.add(
            elements, renderer, &mut self.header, output.band(0.0, grid.top), 1, false,
            Revision::fixed(version, minute), 1.0, 0.0,
            |cr| home.paint_header(cr, layout, time),
        );
        let heading_height = layout.y(if count == 0 { 340.0 } else { 200.0 });
        if (games || count == 0) && offset < heading_height {
            output.add(
                elements, renderer, &mut self.library_heading,
                output.band(grid.top, grid.top + heading_height), 1, false,
                Revision::fixed(version, 0), 1.0, 0.0,
                |cr| {
                    home.paint_library_heading(cr, grid);
                    Ok(())
                },
            );
        }
        let visible = grid.visible(count, offset);
        self.library_tiles.retain(|index, _| visible.contains(index));
        for index in visible {
            let entry = home.library_entries()[index].clone();
            let art = entry.image.as_deref().and_then(|path| home.art(path));
            let alpha = home.library.opacity(now, index, games);
            if alpha <= 0.0 {
                continue;
            }
            let focused = index == home.library.selected && home.header_focus.is_none();
            let detail = u64::from(art.is_some()) | (u64::from(focused) << 1);
            let layer = self.library_tiles.entry(index).or_insert_with(|| Layer::new((side, side), false));
            if let Err(error) = layer.upload(Revision::fixed(library_revision, detail), |cr| {
                let scale = f64::from(side) / grid.size();
                cr.scale(scale, scale);
                home.paint_library_cell(cr, grid, Rect(0.0, 0.0, grid.size(), grid.size()), &entry, art.as_ref(), focused)
            }) {
                tracing::warn!(%error, "Could not paint library tile");
                continue;
            }

            let Rect(x, y, _, _) = grid.cell(index, offset);
            let last = elements.len();
            output.add_element(
                elements, renderer, layer, ((x * factor).round(), (y * factor).round()),
                (f64::from(side), f64::from(side)), alpha, false, None,
            );
            if elements.len() > last {
                if let Some(CustomRenderElements::Home(tile)) = elements.pop() {
                    if let Some(tile) = CropRenderElement::from_element(Wrap::from(tile), output.scale, viewport) {
                        elements.push(CustomRenderElements::HomeClip(tile));
                    }
                }
            }
        }
        let (background, shade) = home.library_background();
        output.add(
            elements, renderer, &mut self.background, whole, output.backdrop_divisor(background.as_ref()), true,
            Revision::fixed(1, u64::from(background.is_some())), 1.0, 0.0,
            |cr| home.paint_background_art(cr, layout, background.as_ref(), shade),
        );
    }

    fn render_carousel<R>(
        &mut self,
        home: &mut Home,
        renderer: &mut R,
        elements: &mut Vec<CustomRenderElements<R>>,
        output: Output,
        layout: Layout,
        now: Instant,
        entrance: Option<f64>,
        moving: bool,
    ) where
        R: Renderer + ImportAll + ImportMem,
        R::TextureId: Clone + Send + 'static,
    {
        let count = home.entries().len();
        let pose = home.carousel_pose(now);
        let factor = f64::from(output.width) / DESIGN_WIDTH;
        let ring_visible = home.action_focus == 0 && home.header_focus.is_none();
        let page = usize::from(home.page == Page::Apps);
        let normal = &mut self.carousel_tiles[page];
        let large = &mut self.growing_tiles[page];
        normal.resize_with(count + 1, Default::default);
        large.resize_with(count, Default::default);
        // Reuse the tile artwork as it scales. Only the focus outline, label,
        // and library icon are redrawn during the transition.
        let layer_start = elements.len();
        let mut focus_tiles = Vec::new();
        for (index, rect @ Rect(x, y, size, _), grow) in layout.tiles(pose, count, entrance) {
            let entry = home.entries().get(index).cloned();
            if grow > 0.0 {
                focus_tiles.push((index, rect, if ring_visible { grow } else { 0.0 }));
                if entry.is_none() {
                    continue;
                }
            }
            let (cache, base) = if grow > 0.20 { (&mut *large, SELECTED) } else { (&mut *normal, TILE) };
            let icon = match entry.as_ref() {
                Some(entry) => entry.image.clone(),
                None => Some(home.root.join("icons/game-library.png")),
            };
            let art_ready = icon.as_deref().and_then(|path| home.art(path)).is_some();
            let side = ((base * factor).round() as i32).max(1);
            let layer = cache[index].get_or_insert_with(|| Layer::new((side + 4, side + 4), false));
            if let Err(error) = layer.upload(Revision::fixed(u64::from(art_ready), 0), |cr| {
                cr.translate(2.0, 2.0);
                cr.scale(f64::from(side) / base, f64::from(side) / base);
                home.paint_tile_body(cr, Rect(0.0, 0.0, base, base), entry.as_ref(), 0.0)
            }) {
                tracing::warn!(%error, "Could not paint carousel tile");
                continue;
            }
            let (x, y, extent) = (x * factor, y * factor, size * factor);
            let (left, top) = (x.round(), y.round());
            let texels_per_pixel = f64::from(side) / extent;
            let shift = |delta: f64| (delta * texels_per_pixel).clamp(-1.5, 1.5);
            let source = Rectangle::new(
                (2.0 + shift(left - x), 2.0 + shift(top - y)).into(),
                (f64::from(side), f64::from(side)).into(),
            );
            output.add_element(
                elements,
                renderer,
                layer,
                (left, top),
                (extent, extent),
                1.0,
                false,
                Some(source),
            );
        }
        let Some((_, Rect(x, _, size, _), _)) = focus_tiles.first().copied() else {
            return;
        };
        let (left, right) = focus_tiles
            .iter()
            .skip(1)
            .fold((x, x + size), |(left, right), (_, Rect(x, _, size, _), _)| {
                (left.min(*x), right.max(x + size))
            });
        let mut region = output.region(
            left - 80.0,
            layout.y(269.0) - 16.0,
            right + 80.0,
            layout.y(269.0) + 371.0,
        );
        let aligned_width = ((region.width + 127) / 128 * 128).min(output.width);
        region.x = region.x.min(output.width - aligned_width);
        region.width = aligned_width;
        let focus_start = elements.len();
        output.add(
            elements,
            renderer,
            &mut self.carousel_focus,
            region,
            1,
            false,
            Revision {
                state: home.version,
                frame: moving.then_some(now),
                detail: home.art_revision,
            },
            1.0,
            0.0,
            |cr| {
                for (index, rect, ring) in focus_tiles {
                    let entry = home.entries().get(index).cloned();
                    if entry.is_none() {
                        home.paint_tile_body(cr, rect, None, ring)?;
                    }
                    home.paint_tile_focus(cr, rect, entry.as_ref(), ring)?;
                }
                Ok(())
            },
        );
        elements[layer_start..].rotate_left(focus_start - layer_start);
    }
}
