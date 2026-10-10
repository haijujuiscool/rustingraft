mod app;
#[cfg(test)]
mod flow_fixtures;
mod ground;
mod player;
mod save;
mod shared;
mod ui;
mod view;

use rusting_engine::prelude::*;
use std::cell::RefCell;

thread_local! {
    static GAME: RefCell<Option<app::Game>> = const { RefCell::new(None) };
}

/// Runs `f` on the game, creating it first; the tick hook can run before the first frame.
fn with_game(scene: &mut GameScene<'_>, f: impl FnOnce(&mut app::Game, &mut GameScene<'_>)) {
    GAME.with(|cell| {
        let mut game = cell.borrow_mut();
        let game = game.get_or_insert_with(|| app::Game::new(scene));
        f(game, scene);
    });
}

fn tick(scene: &mut GameScene<'_>, _time: &FrameTime) {
    with_game(scene, |game, scene| game.tick(scene));
}

fn update(scene: &mut GameScene<'_>, time: &FrameTime) {
    let dt = time.delta_seconds();
    with_game(scene, |game, scene| game.frame(scene, dt));
}

rusting_game!(update, tick: tick);
