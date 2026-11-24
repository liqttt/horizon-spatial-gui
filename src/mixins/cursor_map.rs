fn cursor_map(game: &mut Game<'_>) {
    game.cursor = mod_spatial_gui::on_cursor_moved(game.cursor.0, game.cursor.1);
}
