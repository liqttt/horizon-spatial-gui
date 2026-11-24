fn hud_head(game: &mut Game<'_>, size: [u32; 2]) {
    let is_screen = match game.screen {
        Screen::Inventory | Screen::Creative => true,
        _ => false,
    };
    let is_container = is_screen
        && game.server_packs.prompt.is_none()
        && game.book.is_none();
    let gui = HudBatch::new(size, game.settings.gui_scale_inputs());
    if let ::core::option::Option::Some(mapped) =
        mod_spatial_gui::on_hud_head(is_container, gui.width, gui.height, game.cursor)
    {
        game.cursor = mapped;
    }
}
