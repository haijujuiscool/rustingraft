//! Barnaby's shop and the wandering trader (spec 12).
//!
//! - Sell { slot } / SellAllFish: within SHOP_RANGE of Spot::Barnaby on Hook Isle; raw
//!   or cooked fish at `ItemKind::sell_price`. Every fish sold counts toward quest 1.
//! - Buy { shop, offer }: Barnaby from BARNABY_STOCK near Spot::Barnaby; the trader from
//!   `Trader::stock` while present, within TRADER_RANGE of `Trader::dock`. Upgrades the
//!   player already has are refused (`can_buy`, also for the UI to grey them out).
//!   Offers never sell out.
//! - `step`: on DayStarted(d) with d % TRADER_EVERY_DAYS == 0 the trader docks at
//!   `Raft::trader_dock_pos` with TRADER_OFFERS_PER_VISIT distinct offers drawn from
//!   TRADER_POOL by the world RNG; it leaves at dusk.

use crate::clock::{phase_entered, Phase};
use crate::content::{
    Goods, ItemKind, Offer, Shop, Spot, BARNABY_STOCK, HOOK_ISLE, SHOP_RANGE, TRADER_EVERY_DAYS,
    TRADER_OFFERS_PER_VISIT, TRADER_POOL, TRADER_RANGE,
};
use crate::islands;
use crate::math::V3;
use crate::msg::{deny, sound, toast, Event, Request, SoundKind};
use crate::players::Player;
use crate::world::{Ground, PlayerId, World};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Trader {
    pub present: bool,
    /// This visit's offers (empty while away), in TRADER_POOL order.
    pub stock: Vec<Offer>,
    /// Where the trader's boat is moored while present.
    pub dock: V3,
}

impl Trader {
    pub fn new() -> Trader {
        Trader::default()
    }
}

/// Why `p` may not buy `goods` (an upgrade it already has), if so.
pub fn can_buy(p: &Player, goods: Goods) -> Result<(), &'static str> {
    let owned = match goods {
        Goods::Sac(c) => p.sac.capacity() >= c as usize,
        Goods::Harpoon(t) => p.gear.harpoon >= t,
        Goods::Melee(t) => p.gear.melee >= t,
        Goods::Rod(t) => p.gear.rod >= t,
        Goods::BarbedTip => p.gear.barbed_tip,
        Goods::LuckyLure => p.gear.lucky_lure,
        Goods::Item(..) | Goods::Coins(_) | Goods::Stock(_) => false,
    };
    if owned {
        Err("You already have that")
    } else {
        Ok(())
    }
}

pub(crate) fn step(w: &mut World, _dt: f32, _g: &dyn Ground, ev: &mut Vec<Event>) {
    let arrives = ev
        .iter()
        .any(|e| matches!(e, Event::DayStarted(d) if d % TRADER_EVERY_DAYS == 0));
    if arrives && !w.trader.present {
        let mut pool: Vec<usize> = (0..TRADER_POOL.len()).collect();
        let mut picked = Vec::with_capacity(TRADER_OFFERS_PER_VISIT);
        for _ in 0..TRADER_OFFERS_PER_VISIT.min(pool.len()) {
            let i = w.rng.range_u32(0, pool.len() as u32 - 1) as usize;
            picked.push(pool.swap_remove(i));
        }
        picked.sort_unstable();
        w.trader = Trader {
            present: true,
            stock: picked.into_iter().map(|i| TRADER_POOL[i]).collect(),
            dock: w.raft.trader_dock_pos(),
        };
        ev.push(Event::TraderArrived);
        sound(ev, SoundKind::TraderHorn, w.trader.dock);
        toast(ev, None, "A trader has docked at the raft");
    } else if w.trader.present && phase_entered(ev, Phase::Dusk) {
        w.trader.present = false;
        w.trader.stock.clear();
        ev.push(Event::TraderLeft);
        toast(ev, None, "The trader sailed off");
    }
}

pub(crate) fn handle(
    w: &mut World,
    pid: PlayerId,
    req: Request,
    _g: &dyn Ground,
    ev: &mut Vec<Event>,
) {
    let Some(p) = w.players.get(&pid) else { return };
    let at_barnaby = w.near_spot(pid, HOOK_ISLE, Spot::Barnaby, SHOP_RANGE);
    let pos = p.pos;
    match req {
        Request::Sell { slot } => {
            if !at_barnaby {
                return deny(ev, pid, "Barnaby buys fish on Hook Isle");
            }
            let Some(stack) = p.sac.get(slot).filter(|s| s.kind.is_fish()) else {
                return deny(ev, pid, "Barnaby only buys fish");
            };
            let p = w.players.get_mut(&pid).expect("checked above");
            p.sac.take(slot);
            sell(w, pid, &[stack], pos, ev);
        }
        Request::SellAllFish => {
            if !at_barnaby {
                return deny(ev, pid, "Barnaby buys fish on Hook Isle");
            }
            if !p.sac.stacks().any(|(_, s)| s.kind.is_fish()) {
                return deny(ev, pid, "You have no fish");
            }
            let p = w.players.get_mut(&pid).expect("checked above");
            let sold = p.sac.take_all(ItemKind::is_fish);
            sell(w, pid, &sold, pos, ev);
        }
        Request::Buy { shop, offer } => {
            let offer = match shop {
                Shop::Barnaby if !at_barnaby => {
                    return deny(ev, pid, "Barnaby's shop is on Hook Isle")
                }
                Shop::Barnaby => BARNABY_STOCK.get(offer).copied(),
                Shop::Trader if !w.trader.present => {
                    return deny(ev, pid, "The trader is not here")
                }
                Shop::Trader if pos.dist_xz(w.trader.dock) > TRADER_RANGE => {
                    return deny(ev, pid, "Walk up to the trader's boat")
                }
                Shop::Trader => w.trader.stock.get(offer).copied(),
            };
            let Some(offer) = offer else {
                return deny(ev, pid, "That is not for sale");
            };
            if let Err(why) = can_buy(p, offer.goods) {
                return deny(ev, pid, why);
            }
            if p.coins < offer.price {
                return deny(ev, pid, format!("You need {} coins", offer.price));
            }
            if !w.give(pid, offer.goods, ev) {
                return deny(ev, pid, "Your sac is full");
            }
            w.players.get_mut(&pid).expect("checked above").coins -= offer.price;
            sound(ev, SoundKind::Coins, pos);
        }
        _ => {}
    }
}

/// Pays for fish already taken out of the sac and counts them for quest 1.
fn sell(
    w: &mut World,
    pid: PlayerId,
    stacks: &[crate::inventory::Stack],
    pos: V3,
    ev: &mut Vec<Event>,
) {
    let count: u32 = stacks.iter().map(|s| s.count).sum();
    let coins: u32 = stacks
        .iter()
        .map(|s| s.count * s.kind.sell_price().unwrap_or(0))
        .sum();
    w.give(pid, Goods::Coins(coins), ev);
    sound(ev, SoundKind::Coins, pos);
    islands::fish_sold(w, count, ev);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{island, Fish, CYCLE_SECONDS, DAWN_SECONDS, DAY_SECONDS, DUSK_SECONDS};
    use crate::world::FlatGround;

    const G: FlatGround = FlatGround::WATER;

    fn denied(ev: &[Event]) -> bool {
        matches!(ev, [Event::Denied { .. }])
    }

    fn at_barnaby() -> (World, PlayerId) {
        let mut w = World::new(3);
        let a = w.join("Ana").unwrap();
        w.player_mut(a).unwrap().pos = island(HOOK_ISLE).spot(Spot::Barnaby) + V3::xz(1.0, 1.0);
        (w, a)
    }

    #[test]
    fn sell_fish_at_barnaby() {
        let (mut w, a) = at_barnaby();
        let p = w.player_mut(a).unwrap();
        p.sac.add(ItemKind::Fish(Fish::Goldfin), 2);
        p.sac.add(ItemKind::CookedFish(Fish::Sardine), 3);
        p.sac.add(ItemKind::Plank, 4);
        assert!(denied(&w.apply(a, Request::Sell { slot: 2 }, &G)), "planks");
        assert!(denied(&w.apply(a, Request::Sell { slot: 7 }, &G)), "empty");
        assert!(denied(&w.apply(a, Request::Sell { slot: usize::MAX }, &G)));
        w.apply(a, Request::Sell { slot: 0 }, &G);
        assert_eq!(w.player(a).unwrap().coins, 70);
        w.apply(a, Request::SellAllFish, &G);
        let p = w.player(a).unwrap();
        assert_eq!(p.coins, 82);
        assert_eq!(p.sac.count(ItemKind::Plank), 4);
        assert!(!p.sac.stacks().any(|(_, s)| s.kind.is_fish()));
        assert!(
            denied(&w.apply(a, Request::SellAllFish, &G)),
            "nothing left"
        );
    }

    #[test]
    fn selling_far_from_barnaby_is_denied() {
        let mut w = World::new(3);
        let a = w.join("Ana").unwrap();
        w.player_mut(a)
            .unwrap()
            .sac
            .add(ItemKind::Fish(Fish::Sardine), 1);
        let before = w.clone();
        assert!(denied(&w.apply(a, Request::Sell { slot: 0 }, &G)));
        assert!(denied(&w.apply(a, Request::SellAllFish, &G)));
        assert!(denied(&w.apply(
            a,
            Request::Buy {
                shop: Shop::Barnaby,
                offer: 3
            },
            &G
        )));
        assert_eq!(w, before);
    }

    #[test]
    fn buy_from_barnaby() {
        let (mut w, a) = at_barnaby();
        let buy = |offer| Request::Buy {
            shop: Shop::Barnaby,
            offer,
        };
        assert!(denied(&w.apply(a, buy(0), &G)), "no coins");
        w.player_mut(a).unwrap().coins = 300;
        assert!(denied(&w.apply(a, buy(4), &G)), "no such offer");
        assert!(!denied(&w.apply(a, buy(0), &G)));
        assert!(!denied(&w.apply(a, buy(2), &G)));
        assert!(!denied(&w.apply(a, buy(3), &G)));
        let p = w.player(a).unwrap();
        assert_eq!((p.coins, p.gear.rod, p.sac.capacity()), (140, 2, 12));
        assert_eq!(p.sac.count(ItemKind::Bait), 5);
        let before = w.clone();
        assert!(denied(&w.apply(a, buy(0), &G)), "rod owned");
        assert!(denied(&w.apply(a, buy(2), &G)), "sac owned");
        assert_eq!(w, before);
    }

    #[test]
    fn full_sac_refuses_items_and_keeps_coins() {
        let (mut w, a) = at_barnaby();
        let p = w.player_mut(a).unwrap();
        p.coins = 100;
        for _ in 0..8 {
            p.sac.add(ItemKind::Flare, 1);
        }
        let before = w.clone();
        assert!(denied(&w.apply(
            a,
            Request::Buy {
                shop: Shop::Barnaby,
                offer: 3
            },
            &G
        )));
        assert_eq!(w, before);
        // Upgrades need no room.
        w.apply(
            a,
            Request::Buy {
                shop: Shop::Barnaby,
                offer: 2,
            },
            &G,
        );
        assert_eq!(w.player(a).unwrap().sac.capacity(), 12);
    }

    fn run(w: &mut World, seconds: f32) -> Vec<Event> {
        let mut ev = Vec::new();
        for _ in 0..(seconds * 4.0) as usize {
            ev.extend(w.step(0.25, &G));
        }
        ev
    }

    #[test]
    fn trader_schedule() {
        let mut w = World::new(3);
        assert!(!w.trader.present);
        // Day 2 dawn: arrives.
        w.clock.skip_to(CYCLE_SECONDS - 1.0);
        let ev = run(&mut w, 2.0);
        assert!(ev.contains(&Event::TraderArrived));
        assert_eq!(w.clock.day, 2);
        let t = w.trader.clone();
        assert!(t.present);
        assert_eq!(t.dock, w.raft.trader_dock_pos());
        assert_eq!(t.stock.len(), TRADER_OFFERS_PER_VISIT);
        for (i, o) in t.stock.iter().enumerate() {
            assert!(TRADER_POOL.contains(o));
            assert!(!t.stock[i + 1..].contains(o), "distinct");
        }
        // Still there through the day, gone at dusk.
        let ev = run(&mut w, DAY_SECONDS - DUSK_SECONDS - DAWN_SECONDS);
        assert!(!ev.contains(&Event::TraderLeft) && w.trader.present);
        let ev = run(&mut w, DAWN_SECONDS + 1.0);
        assert!(ev.contains(&Event::TraderLeft));
        assert!(!w.trader.present && w.trader.stock.is_empty());
        // Day 3: no visit. Day 4: a new visit.
        let ev = run(&mut w, CYCLE_SECONDS);
        assert_eq!(w.clock.day, 3);
        assert!(!ev.contains(&Event::TraderArrived));
        let ev = run(&mut w, CYCLE_SECONDS);
        assert_eq!(w.clock.day, 4);
        assert!(ev.contains(&Event::TraderArrived));
    }

    #[test]
    fn trader_stock_varies_with_the_rng() {
        let stocks: std::collections::BTreeSet<Vec<usize>> = (0..20)
            .map(|seed| {
                let mut w = World::new(seed);
                w.clock.skip_to(CYCLE_SECONDS - 0.5);
                run(&mut w, 1.0);
                w.trader
                    .stock
                    .iter()
                    .map(|o| TRADER_POOL.iter().position(|p| p == o).unwrap())
                    .collect()
            })
            .collect();
        assert!(stocks.len() > 5, "{stocks:?}");
    }

    #[test]
    fn buy_from_trader() {
        let mut w = World::new(3);
        let a = w.join("Ana").unwrap();
        w.player_mut(a).unwrap().coins = 2000;
        w.trader = Trader {
            present: true,
            stock: TRADER_POOL[..5].to_vec(),
            dock: w.raft.trader_dock_pos(),
        };
        let buy = |offer| Request::Buy {
            shop: Shop::Trader,
            offer,
        };
        w.player_mut(a).unwrap().pos = w.trader.dock + V3::xz(-12.0, 0.0);
        assert!(denied(&w.apply(a, buy(0), &G)), "too far");
        w.player_mut(a).unwrap().pos = w.trader.dock + V3::xz(-6.0, 0.0);
        assert!(denied(&w.apply(a, buy(5), &G)), "not in this visit's stock");
        assert!(!denied(&w.apply(a, buy(1), &G)), "Sac 24");
        assert!(denied(&w.apply(a, buy(0), &G)), "Sac 16 is smaller than 24");
        assert!(denied(&w.apply(a, buy(1), &G)), "Sac 24 owned");
        assert!(!denied(&w.apply(a, buy(2), &G)), "Sac 32");
        assert!(!denied(&w.apply(a, buy(3), &G)));
        assert!(
            !denied(&w.apply(a, buy(3), &G)),
            "consumables can be bought again"
        );
        let p = w.player(a).unwrap();
        assert_eq!(p.coins, 2000 - 400 - 900 - 25 - 25);
        assert_eq!((p.sac.capacity(), p.sac.count(ItemKind::Bait)), (32, 10));
        w.trader.present = false;
        assert!(denied(&w.apply(a, buy(4), &G)), "gone");
    }

    #[test]
    fn can_buy_rules() {
        let mut p = Player::new("x", V3::ZERO);
        assert!(can_buy(&p, Goods::Sac(12)).is_ok());
        assert!(can_buy(&p, Goods::Sac(8)).is_err());
        assert!(can_buy(&p, Goods::Harpoon(1)).is_err());
        assert!(can_buy(&p, Goods::LuckyLure).is_ok());
        p.gear.lucky_lure = true;
        p.gear.barbed_tip = true;
        assert!(can_buy(&p, Goods::LuckyLure).is_err());
        assert!(can_buy(&p, Goods::BarbedTip).is_err());
        assert!(can_buy(&p, Goods::Item(ItemKind::Bait, 5)).is_ok());
    }
}
