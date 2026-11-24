fn hud_transform(batch: &mut HudBatch) {
    let ::core::option::Option::Some((plane, gui_w, gui_h, subdivide, drop_shadow)) =
        mod_spatial_gui::active_plane()
    else {
        return;
    };
    let start = (batch.blur_at as usize).min(batch.vertices.len());
    if start >= batch.vertices.len() {
        return;
    }

    let mut verts: ::std::vec::Vec<mod_spatial_gui::transform::GuiVert> = batch
        .vertices
        .iter()
        .map(|v| mod_spatial_gui::transform::GuiVert {
            pos: v.pos,
            uv: v.uv,
            color: v.color,
            depth: v.depth,
            glint_uv: v.glint_uv,
            text: v.text,
        })
        .collect();

    let mut tags: ::std::vec::Vec<(::core::ops::Range<u32>, mod_spatial_gui::transform::BlendTag)> =
        batch
            .blends
            .iter()
            .map(|(range, blend)| {
                let tag = match blend {
                    horizon_client_render::hud::Blend::Depth(sc) => {
                        mod_spatial_gui::transform::BlendTag::Depth(*sc)
                    }
                    horizon_client_render::hud::Blend::DepthGlint(sc) => {
                        mod_spatial_gui::transform::BlendTag::DepthGlint(*sc)
                    }
                    _ => mod_spatial_gui::transform::BlendTag::Other,
                };
                (range.clone(), tag)
            })
            .collect();

    mod_spatial_gui::transform::transform_screen_range(
        &mut verts,
        &mut tags,
        start,
        gui_w,
        gui_h,
        batch.scale,
        &plane,
        subdivide,
        drop_shadow,
    );

    batch.vertices = verts
        .into_iter()
        .map(|v| horizon_client_render::hud::HudVertex {
            pos: v.pos,
            uv: v.uv,
            color: v.color,
            depth: v.depth,
            glint_uv: v.glint_uv,
            text: v.text,
        })
        .collect();

    for ((range, blend), (new_range, tag)) in batch.blends.iter_mut().zip(tags.into_iter()) {
        *range = new_range;
        match (blend, tag) {
            (
                horizon_client_render::hud::Blend::Depth(sc),
                mod_spatial_gui::transform::BlendTag::Depth(new_sc),
            ) => *sc = new_sc,
            (
                horizon_client_render::hud::Blend::DepthGlint(sc),
                mod_spatial_gui::transform::BlendTag::DepthGlint(new_sc),
            ) => *sc = new_sc,
            _ => {}
        }
    }
}
