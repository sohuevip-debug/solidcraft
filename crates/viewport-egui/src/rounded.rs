//! Clip both egui paint and GPU pictures to a card without painting over the view behind it.

use std::sync::Arc;

use egui::{CornerRadius, Painter, Rect, Shape, layers::ShapeIdx, pos2};

/// Clip the shapes painted since `start` to the same rounded boundary as the card's chrome.
/// Horizontal bands follow physical pixel rows, keeping adjacent scissor rectangles seamless.
/// GPU pictures are prepared once and only their final composition is clipped.
pub fn clip_card(painter: &Painter, start: ShapeIdx, rect: Rect, radius: CornerRadius) {
    let bands: Arc<[Rect]> = bands(rect, radius, painter.ctx().pixels_per_point()).into();
    painter.ctx().graphics_mut(|graphics| {
        let list = graphics.entry(painter.layer_id());
        let shapes: Vec<_> = list.all_entries().skip(start.0).cloned().collect();
        for (offset, entry) in shapes.iter().enumerate() {
            list.reset_shape(ShapeIdx(start.0 + offset));
            if let Shape::Callback(callback) = &entry.shape {
                // Paint callbacks are opaque to this crate: egui-wgpu intentionally keeps its
                // callback trait object private. Preserve the original callback and split only
                // its clip rectangle; egui-wgpu applies that scissor before each paint call.
                // Tile/highlight callbacks share a per-frame prepare gate, so these paint bands
                // never repeat their GPU preparation work.
                for band in bands.iter() {
                    let clip = band.intersect(entry.clip_rect);
                    if clip.is_positive() && clip.intersects(callback.rect) {
                        list.add(clip, entry.shape.clone());
                    }
                }
                continue;
            }
            let bounds = entry.shape.visual_bounding_rect();
            for band in bands.iter() {
                let clip = band.intersect(entry.clip_rect);
                if clip.is_positive() && clip.intersects(bounds) {
                    list.add(clip, entry.shape.clone());
                }
            }
        }
    });
}

fn bands(rect: Rect, radius: CornerRadius, scale: f32) -> Vec<Rect> {
    if !rect.is_positive() || !rect.is_finite() || !scale.is_finite() || scale <= 0.0 {
        return Vec::new();
    }
    let max_radius = rect.width().min(rect.height()) * 0.5;
    let [nw, ne, sw, se] = [radius.nw, radius.ne, radius.sw, radius.se].map(|r| f32::from(r).min(max_radius));
    let top = nw.max(ne);
    let bottom = sw.max(se);
    let inset = |radius: f32, distance: f32| {
        let dy = (radius - distance).max(0.0);
        radius - (radius * radius - dy * dy).max(0.0).sqrt()
    };
    // Step integer physical rows independently. Multiplying the previous f32
    // boundary by the scale can round below that row and repeat it forever.
    let top_end = (rect.top() + top).min(rect.bottom());
    let bottom_start = (rect.bottom() - bottom).max(top_end);
    let mut boundaries = vec![rect.top()];
    for (start, end) in [(rect.top(), top_end), (bottom_start, rect.bottom())] {
        if start > boundaries.last().copied().unwrap_or(rect.top()) {
            boundaries.push(start);
        }
        if end <= start {
            continue;
        }
        let scale = f64::from(scale);
        let first = (f64::from(start) * scale).floor();
        let count = (f64::from(end) * scale).ceil() - first;
        // Keep hostile scales bounded. Normal DPI uses one band per pixel row.
        let steps = count.clamp(1.0, 512.0) as usize;
        for row in 1..steps {
            let edge = ((first + row as f64 * count / steps as f64) / scale) as f32;
            if edge > boundaries.last().copied().unwrap_or(start) && edge < end {
                boundaries.push(edge);
            }
        }
        boundaries.push(end);
    }
    let mut result = Vec::new();
    for pair in boundaries.windows(2) {
        let [y, end] = [pair[0], pair[1]];
        let middle = (y + end) * 0.5;
        let left = inset(nw, middle - rect.top()).max(inset(sw, rect.bottom() - middle));
        let right = inset(ne, middle - rect.top()).max(inset(se, rect.bottom() - middle));
        result.push(Rect::from_min_max(pos2(rect.left() + left, y), pos2(rect.right() - right, end)));
    }
    result
}
