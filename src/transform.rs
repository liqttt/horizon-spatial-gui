//! World-space 3D GUI vertex projection, drop shadow generation, quad subdivision, and scissor remapping.

use crate::math::WorldPlane;
use std::ops::Range;

/// Minimal vertex representation decoupled from `horizon_client_render` so unit tests run standalone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuiVert {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: u32,
    pub depth: f32,
    pub glint_uv: [f32; 2],
    pub text: u32,
}

/// Minimal blend-run representation matching `HudBatch::blends`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlendTag {
    Other,
    Depth([u32; 4]),
    DepthGlint([u32; 4]),
}

/// Returns true if a 6-vertex quad covers nearly the entire GUI viewport (full-screen dim backdrop).
pub fn is_fullscreen_backdrop(corners: [[f32; 2]; 4], gui_w: f32, gui_h: f32) -> bool {
    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for [x, y] in corners {
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    (max_x - min_x) >= gui_w * 0.95 && (max_y - min_y) >= gui_h * 0.95
}

fn extract_quad(slice: &[GuiVert]) -> Option<[GuiVert; 4]> {
    if slice.len() < 6 {
        return None;
    }
    let eq = |a: &GuiVert, b: &GuiVert| {
        a.pos == b.pos && a.uv == b.uv && a.color == b.color && (a.depth - b.depth).abs() <= 1e-5
    };
    if eq(&slice[0], &slice[3]) && eq(&slice[2], &slice[4]) {
        Some([slice[0], slice[1], slice[2], slice[5]])
    } else if eq(&slice[2], &slice[3]) && eq(&slice[1], &slice[4]) {
        Some([slice[0], slice[1], slice[5], slice[2]])
    } else {
        None
    }
}

#[inline]
fn lerp_f32(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline]
fn lerp_color(c0: u32, c1: u32, t: f32) -> u32 {
    let mut out = 0u32;
    for shift in [0, 8, 16, 24] {
        let a = ((c0 >> shift) & 0xFF) as f32;
        let b = ((c1 >> shift) & 0xFF) as f32;
        let v = (a + (b - a) * t).round().clamp(0.0, 255.0) as u32;
        out |= v << shift;
    }
    out
}

fn bilerp_vert(q: &[GuiVert; 4], u: f32, v: f32) -> GuiVert {
    let top_pos = [lerp_f32(q[0].pos[0], q[1].pos[0], u), lerp_f32(q[0].pos[1], q[1].pos[1], u)];
    let bot_pos = [lerp_f32(q[3].pos[0], q[2].pos[0], u), lerp_f32(q[3].pos[1], q[2].pos[1], u)];
    let top_uv = [lerp_f32(q[0].uv[0], q[1].uv[0], u), lerp_f32(q[0].uv[1], q[1].uv[1], u)];
    let bot_uv = [lerp_f32(q[3].uv[0], q[2].uv[0], u), lerp_f32(q[3].uv[1], q[2].uv[1], u)];
    let top_guv = [
        lerp_f32(q[0].glint_uv[0], q[1].glint_uv[0], u),
        lerp_f32(q[0].glint_uv[1], q[1].glint_uv[1], u),
    ];
    let bot_guv = [
        lerp_f32(q[3].glint_uv[0], q[2].glint_uv[0], u),
        lerp_f32(q[3].glint_uv[1], q[2].glint_uv[1], u),
    ];
    let top_col = lerp_color(q[0].color, q[1].color, u);
    let bot_col = lerp_color(q[3].color, q[2].color, u);

    GuiVert {
        pos: [lerp_f32(top_pos[0], bot_pos[0], v), lerp_f32(top_pos[1], bot_pos[1], v)],
        uv: [lerp_f32(top_uv[0], bot_uv[0], v), lerp_f32(top_uv[1], bot_uv[1], v)],
        color: lerp_color(top_col, bot_col, v),
        depth: lerp_f32(lerp_f32(q[0].depth, q[1].depth, u), lerp_f32(q[3].depth, q[2].depth, u), v),
        glint_uv: [lerp_f32(top_guv[0], bot_guv[0], v), lerp_f32(top_guv[1], bot_guv[1], v)],
        text: q[0].text,
    }
}

#[inline]
fn project_vert(plane: &WorldPlane, mut v: GuiVert) -> GuiVert {
    if let Some((px, py)) = plane.project(v.pos[0], v.pos[1]) {
        v.pos = [px, py];
    }
    v
}

#[inline]
fn project_shadow_vert(plane: &WorldPlane, mut v: GuiVert) -> GuiVert {
    if let Some((px, py)) = plane.project_shadow(v.pos[0], v.pos[1]) {
        v.pos = [px, py];
    }
    // Preserve UV silhouette (so tabs/rounded corners match!) and tint black at ~40% alpha.
    let alpha = ((v.color >> 24) & 0xFF) * 100 / 255;
    v.color = alpha << 24;
    v
}

/// Projects physical-pixel scissor `[x, y, w, h]` through `plane` and returns an expanded clamped AABB.
pub fn transform_scissor(
    scissor: [u32; 4],
    scale: f32,
    plane: &WorldPlane,
    gui_w: f32,
    gui_h: f32,
) -> [u32; 4] {
    let inv_scale = if scale > 0.0 { 1.0 / scale } else { 1.0 };
    let gx0 = scissor[0] as f32 * inv_scale;
    let gy0 = scissor[1] as f32 * inv_scale;
    let gx1 = (scissor[0] + scissor[2]) as f32 * inv_scale;
    let gy1 = (scissor[1] + scissor[3]) as f32 * inv_scale;

    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;

    for (cx, cy) in [(gx0, gy0), (gx1, gy0), (gx1, gy1), (gx0, gy1)] {
        let (px, py) = plane.project(cx, cy).unwrap_or((cx, cy));
        min_x = min_x.min(px * scale);
        max_x = max_x.max(px * scale);
        min_y = min_y.min(py * scale);
        max_y = max_y.max(py * scale);
    }

    let fb_w = (gui_w * scale).ceil().max(1.0) as u32;
    let fb_h = (gui_h * scale).ceil().max(1.0) as u32;
    let x = min_x.floor().max(0.0) as u32;
    let y = min_y.floor().max(0.0) as u32;
    let x_end = (max_x.ceil().max(0.0) as u32).min(fb_w);
    let y_end = (max_y.ceil().max(0.0) as u32).min(fb_h);
    [x, y, x_end.saturating_sub(x), y_end.saturating_sub(y)]
}

/// Projects screen vertices (`start_idx..`) onto `plane`, adding optional 3D drop shadows behind
/// panel quads, subdividing large 2D quads, and remapping `blends` + scissor rectangles.
pub fn transform_screen_range(
    vertices: &mut Vec<GuiVert>,
    blends: &mut [(Range<u32>, BlendTag)],
    start_idx: usize,
    gui_w: f32,
    gui_h: f32,
    scale: f32,
    plane: &WorldPlane,
    subdivide: bool,
    drop_shadow: bool,
) {
    let end_idx = vertices.len();
    if start_idx >= end_idx {
        return;
    }

    let tail_len = end_idx - start_idx;
    let mut is_depth_vert = vec![false; tail_len];
    for (range, tag) in blends.iter() {
        if matches!(tag, BlendTag::Depth(_) | BlendTag::DepthGlint(_)) && range.end > start_idx as u32 {
            let s = (range.start as usize).saturating_sub(start_idx).min(tail_len);
            let e = (range.end as usize).saturating_sub(start_idx).min(tail_len);
            for flag in &mut is_depth_vert[s..e] {
                *flag = true;
            }
        }
    }

    // Optional 3D silhouette drop shadow for large non-text, non-backdrop panel quads
    let mut shadow_tail: Vec<GuiVert> = Vec::new();
    if drop_shadow {
        let mut i = start_idx;
        while i + 6 <= end_idx {
            let rel = i - start_idx;
            let can_quad = !is_depth_vert[rel..rel + 6].iter().any(|&d| d);
            if can_quad
                && let Some(quad) = extract_quad(&vertices[i..i + 6])
                && quad[0].text == 0
                && !is_fullscreen_backdrop([quad[0].pos, quad[1].pos, quad[2].pos, quad[3].pos], gui_w, gui_h)
            {
                let w = (quad[1].pos[0] - quad[0].pos[0]).abs();
                let h = (quad[3].pos[1] - quad[0].pos[1]).abs();
                if w >= 20.0 && h >= 20.0 {
                    for v in &vertices[i..i + 6] {
                        shadow_tail.push(project_shadow_vert(plane, *v));
                    }
                }
                i += 6;
            } else {
                i += 1;
            }
        }
    }

    let shadow_len = shadow_tail.len();
    let base_out = start_idx + shadow_len;
    let mut out_tail: Vec<GuiVert> = Vec::with_capacity(shadow_len + tail_len + 64);
    out_tail.extend_from_slice(&shadow_tail);

    let mut new_index_of = Vec::with_capacity(tail_len + 1);
    let mut i = start_idx;
    while i < end_idx {
        let rel = i - start_idx;
        let can_quad = i + 6 <= end_idx && !is_depth_vert[rel..rel + 6].iter().any(|&d| d);
        if can_quad
            && let Some(quad) = extract_quad(&vertices[i..i + 6])
        {
            let corners = [quad[0].pos, quad[1].pos, quad[2].pos, quad[3].pos];
            if is_fullscreen_backdrop(corners, gui_w, gui_h) {
                for v in &vertices[i..i + 6] {
                    new_index_of.push((start_idx + out_tail.len()) as u32);
                    out_tail.push(*v);
                }
                i += 6;
                continue;
            }

            let w = (quad[1].pos[0] - quad[0].pos[0]).abs().max((quad[2].pos[0] - quad[3].pos[0]).abs());
            let h = (quad[3].pos[1] - quad[0].pos[1]).abs().max((quad[2].pos[1] - quad[1].pos[1]).abs());
            let (nx, ny) = if subdivide && (w > 24.0 || h > 24.0) {
                (
                    ((w / 24.0).ceil() as usize).clamp(1, 8),
                    ((h / 24.0).ceil() as usize).clamp(1, 8),
                )
            } else {
                (1, 1)
            };

            let quad_out_start = (start_idx + out_tail.len()) as u32;
            for _ in 0..6 {
                new_index_of.push(quad_out_start);
            }

            if nx == 1 && ny == 1 {
                for v in &vertices[i..i + 6] {
                    out_tail.push(project_vert(plane, *v));
                }
            } else {
                for iy in 0..ny {
                    let v0 = iy as f32 / ny as f32;
                    let v1 = (iy + 1) as f32 / ny as f32;
                    for ix in 0..nx {
                        let u0 = ix as f32 / nx as f32;
                        let u1 = (ix + 1) as f32 / nx as f32;
                        let p00 = project_vert(plane, bilerp_vert(&quad, u0, v0));
                        let p10 = project_vert(plane, bilerp_vert(&quad, u1, v0));
                        let p11 = project_vert(plane, bilerp_vert(&quad, u1, v1));
                        let p01 = project_vert(plane, bilerp_vert(&quad, u0, v1));
                        out_tail.extend_from_slice(&[p00, p10, p11, p00, p11, p01]);
                    }
                }
            }
            i += 6;
            continue;
        }

        new_index_of.push((start_idx + out_tail.len()) as u32);
        out_tail.push(project_vert(plane, vertices[i]));
        i += 1;
    }
    new_index_of.push((start_idx + out_tail.len()) as u32);

    vertices.truncate(start_idx);
    vertices.extend_from_slice(&out_tail);

    let map_idx = |old: u32| -> u32 {
        let idx = old as usize;
        if idx <= start_idx {
            old
        } else {
            let rel = (idx - start_idx).min(tail_len);
            new_index_of[rel]
        }
    };

    for (range, tag) in blends.iter_mut() {
        let s = map_idx(range.start);
        let e = map_idx(range.end);
        *range = s..e;
        if range.end > base_out as u32 {
            match tag {
                BlendTag::Depth(sc) => *sc = transform_scissor(*sc, scale, plane, gui_w, gui_h),
                BlendTag::DepthGlint(sc) => *sc = transform_scissor(*sc, scale, plane, gui_w, gui_h),
                BlendTag::Other => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::SpatialRuntime;
    use mod_api::CameraType;

    fn make_quad(x: f32, y: f32, w: f32, h: f32) -> [GuiVert; 6] {
        let v = |px, py, u, v| GuiVert {
            pos: [px, py],
            uv: [u, v],
            color: u32::MAX,
            depth: 0.0,
            glint_uv: [0.0, 0.0],
            text: 0,
        };
        let v0 = v(x, y, 0.0, 0.0);
        let v1 = v(x + w, y, 1.0, 0.0);
        let v2 = v(x + w, y + h, 1.0, 1.0);
        let v3 = v(x, y + h, 0.0, 1.0);
        [v0, v1, v2, v0, v2, v3]
    }

    #[test]
    fn world_transform_subdivides_adds_shadow_and_remaps_depth_runs() {
        let mut rt = SpatialRuntime::default();
        rt.record_camera_view(true, [0.0, 65.62, 0.0], 1.62, [0.0, 0.0]);
        rt.compute_camera_pose(1.0, [0.0, 65.62, 0.0], 0.0, 0.0, CameraType::FirstPerson, 70.0);
        rt.step_hud(true, 480.0, 270.0, (240.0, 135.0));
        let plane = rt.current_plane.expect("plane");

        let mut verts = Vec::new();
        verts.extend_from_slice(&make_quad(10.0, 10.0, 20.0, 20.0)); // HUD 0..6
        let blur_at = verts.len();
        verts.extend_from_slice(&make_quad(152.0, 52.0, 176.0, 166.0)); // panel 6..12
        verts.extend_from_slice(&make_quad(160.0, 60.0, 48.0, 48.0)); // 3D depth run 12..18

        let mut blends = vec![
            (0..6, BlendTag::Other),
            (6..12, BlendTag::Other),
            (12..18, BlendTag::Depth([304, 104, 96, 96])),
        ];
        transform_screen_range(&mut verts, &mut blends, blur_at, 480.0, 270.0, 2.0, &plane, true, true);

        // 6 HUD + 6 shadow (panel) + 336 subdivided panel + 6 unsubdivided depth = 354
        assert_eq!(verts.len(), 6 + 6 + 336 + 6);
        assert_eq!(blends[0].0, 0..6);
        assert_eq!(blends[1].0, 6..348);
        assert_eq!(blends[2].0, 348..354);
    }
}
