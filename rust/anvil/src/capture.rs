//! On-demand framebuffer capture. Encoding and file I/O stay off the render loop.
use std::{
    fs::{self, OpenOptions},
    io::{self, BufWriter, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::mpsc::{self, SyncSender},
    thread,
};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{ExportMem, TextureMapping},
    },
    reexports::calloop::channel::{channel, Event},
    utils::{Buffer, Rectangle, Size, Transform},
};

use crate::{state::Backend, AnvilState};

#[derive(Default, Debug)]
pub(crate) struct Capture {
    pub(super) pending: Option<String>,
    writer: Option<SyncSender<Frame>>,
}

struct Frame {
    bytes: Vec<u8>,
    size: (u32, u32),
    flipped: bool,
    transform: Transform,
    group: String,
}

pub(super) fn install<B: Backend + 'static>(state: &mut AnvilState<B>) {
    let (updates, receive) = channel();
    if state
        .handle
        .insert_source(receive, |event, _, state| {
            if let Event::Msg(result) = event {
                state.home.capture_complete(result);
            }
        })
        .is_err()
    {
        return;
    }
    let root = state.home.root.join("state/screenshots");
    let (send, frames) = mpsc::sync_channel(1);
    state.capture.writer = Some(send);
    thread::spawn(move || {
        if let Some(path) = latest_shot(&root) {
            let _ = updates.send(Ok(Some(path)));
        }
        for frame in frames {
            let result = encode(&root, frame).map(Some).map_err(|error| error.to_string());
            if updates.send(result).is_err() {
                break;
            }
        }
    });
}

impl Capture {
    pub(super) fn queue(&mut self, group: String) {
        if self.writer.is_some() {
            self.pending = Some(group);
        }
    }

    pub(super) fn read<R: ExportMem>(
        &mut self,
        renderer: &mut R,
        framebuffer: &R::Framebuffer<'_>,
        size: Size<i32, Buffer>,
        transform: Transform,
    ) -> Result<(), String> {
        let Some(group) = self.pending.take() else {
            return Ok(());
        };
        let Some(writer) = &self.writer else {
            return Ok(());
        };
        let mapping = renderer
            .copy_framebuffer(framebuffer, Rectangle::from_size(size), Fourcc::Abgr8888)
            .map_err(|error| error.to_string())?;
        let flipped = mapping.flipped();
        let bytes = renderer
            .map_texture(&mapping)
            .map_err(|error| error.to_string())?
            .to_vec();
        writer
            .try_send(Frame {
                bytes,
                size: (size.w as u32, size.h as u32),
                flipped,
                transform,
                group,
            })
            .map_err(|_| "A screenshot is already being saved.".into())
    }
}

fn encode(root: &Path, frame: Frame) -> io::Result<PathBuf> {
    let dir = root.join(frame.group);
    fs::create_dir_all(&dir)?;
    let name = format!("{}.png", chrono::Local::now().format("%Y-%m-%d_%H-%M-%S-%f"));
    let path = dir.join(name);
    let scratch = path.with_extension("part");
    let (width, height) = frame.size;
    let Some(mut image) = image::RgbaImage::from_vec(width, height, frame.bytes) else {
        return Err(io::Error::other("Invalid framebuffer length"));
    };
    if frame.flipped {
        image::imageops::flip_vertical_in_place(&mut image);
    }
    for pixel in image.pixels_mut() {
        pixel.0[3] = 255;
    }
    let mut image = image::DynamicImage::ImageRgba8(image);
    if matches!(
        frame.transform,
        Transform::Flipped | Transform::Flipped90 | Transform::Flipped180 | Transform::Flipped270
    ) {
        image = image.fliph();
    }
    image = match frame.transform {
        Transform::_90 | Transform::Flipped90 => image.rotate270(),
        Transform::_180 | Transform::Flipped180 => image.rotate180(),
        Transform::_270 | Transform::Flipped270 => image.rotate90(),
        _ => image,
    };
    let result = (|| {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&scratch)?;
        let mut writer = BufWriter::new(file);
        image
            .write_to(&mut writer, image::ImageFormat::Png)
            .map_err(io::Error::other)?;
        writer.flush()?;
        fs::rename(&scratch, &path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&scratch);
    }
    result.map(|()| path)
}

fn latest_shot(root: &Path) -> Option<PathBuf> {
    let mut latest = None;
    let mut modified = None;
    for entry in fs::read_dir(root).ok()?.flatten() {
        let path = entry.path();
        let paths = if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            fs::read_dir(path)
                .ok()
                .into_iter()
                .flatten()
                .flatten()
                .map(|child| child.path())
                .collect()
        } else {
            vec![path]
        };
        for path in paths {
            if path.extension().and_then(|ext| ext.to_str()) != Some("png") {
                continue;
            }
            let Some(time) = path.metadata().and_then(|meta| meta.modified()).ok() else {
                continue;
            };
            if modified.is_none_or(|last| time > last) {
                latest = Some(path);
                modified = Some(time);
            }
        }
    }
    latest
}
