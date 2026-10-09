//! Wire messages (JSON inside relay frames) and who receives each event.

use raft_sim::{DynamicSnapshot, Event, PlayerId, Request, SectionId, World};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Client -> host.
#[derive(Debug, Serialize, Deserialize)]
pub enum ToHost {
    /// First message after the relay join.
    Hello {
        version: u32,
        name: String,
    },
    Req(Request),
    /// Throttled movement: `PlayerTransform` or `BoatTransform`.
    Pose(Request),
}

/// Host -> client.
#[derive(Debug, Serialize, Deserialize)]
pub enum ToClient {
    /// Also sent again after `Session::replace_world`.
    Welcome {
        player_id: PlayerId,
        world: Box<World>,
    },
    Reject {
        reason: String,
    },
    /// A slow section that changed (broadcast).
    Section {
        id: SectionId,
        value: Value,
    },
    /// The receiver's own full player entry.
    Me(Value),
    Snapshot(DynamicSnapshot),
    Events(Vec<Event>),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Audience {
    All,
    Player(PlayerId),
    /// Only the host acts on it.
    Host,
}

impl Audience {
    pub fn includes(&self, pid: PlayerId) -> bool {
        match self {
            Audience::All => true,
            Audience::Player(p) => *p == pid,
            Audience::Host => false,
        }
    }
}

/// Personal feedback goes to its player; anything others can see or hear goes to all.
/// No wildcard arm: a new `Event` variant must be placed here before the crate builds.
pub fn audience(e: &Event) -> Audience {
    use Event as E;
    match e {
        E::Denied { player, .. }
        | E::Gained { player, .. }
        | E::PlayerTeleport { player, .. }
        | E::FishBite { player, .. }
        | E::Toast {
            player: Some(player),
            ..
        } => Audience::Player(*player),
        // The host writes the save file; the payer gets the broadcast toast.
        E::SaveRequested { .. } => Audience::Host,
        E::Toast { player: None, .. }
        | E::Sound { .. }
        | E::Damage { .. }
        | E::PlayerDowned(_)
        | E::PlayerRevived(_)
        | E::GameOver
        | E::PhaseChanged(_)
        | E::DayStarted(_)
        | E::NightStarted
        | E::CampfireLevelUp(_)
        | E::CampfireOut
        | E::EnemySpawned { .. }
        | E::EnemyKilled { .. }
        | E::TraderArrived
        | E::TraderLeft
        | E::IslandDiscovered(_)
        | E::QuestUpdated(_)
        | E::SurvivorSignal(_)
        | E::SurvivorRescued(_)
        | E::EndingReached => Audience::All,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raft_sim::{SoundKind, V3};

    #[test]
    fn personal_events_reach_only_their_player() {
        let denied = Event::Denied {
            player: 2,
            reason: "no".into(),
        };
        assert_eq!(audience(&denied), Audience::Player(2));
        assert!(audience(&denied).includes(2) && !audience(&denied).includes(1));
        let toast = |player| Event::Toast {
            player,
            text: "hi".into(),
        };
        assert_eq!(audience(&toast(Some(3))), Audience::Player(3));
        assert_eq!(audience(&toast(None)), Audience::All);
        let sound = Event::Sound {
            kind: SoundKind::Splash,
            pos: V3::ZERO,
        };
        assert_eq!(audience(&sound), Audience::All);
        assert_eq!(audience(&Event::PlayerDowned(4)), Audience::All);
        assert_eq!(audience(&Event::SaveRequested { by: 1 }), Audience::Host);
        assert!(!Audience::Host.includes(1));
    }

    #[test]
    fn messages_round_trip_as_json() {
        let msg = ToHost::Hello {
            version: 7,
            name: "Ana".into(),
        };
        let back: ToHost = serde_json::from_slice(&serde_json::to_vec(&msg).unwrap()).unwrap();
        assert!(matches!(back, ToHost::Hello { version: 7, name } if name == "Ana"));
        let msg = ToClient::Welcome {
            player_id: 3,
            world: Box::new(World::new(4)),
        };
        let back: ToClient = serde_json::from_slice(&serde_json::to_vec(&msg).unwrap()).unwrap();
        assert!(
            matches!(back, ToClient::Welcome { player_id: 3, world } if *world == World::new(4))
        );
    }
}
