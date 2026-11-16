mod blackjack;
mod card;
mod partnership;
mod poker;
mod rummy;
mod solitaire;
mod trick;

use prost::Message;
use serde::Deserialize;
use std::{
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr, slice,
};

mod wire {
    include!(concat!(env!("OUT_DIR"), "/old_james.cards.rs"));
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ruleset {
    version: u32,
    name: String,
    summary: String,
    artwork: Artwork,
    rules: Rules,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Artwork {
    Poker,
    Bridge,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Rules {
    Solitaire(solitaire::Rules),
    Trick(trick::Rules),
    #[serde(alias = "holdem")]
    Poker(poker::Rules),
    Blackjack(blackjack::Rules),
    Rummy(rummy::Rules),
    Partnership(partnership::Rules),
}

impl Ruleset {
    fn read(json: &str) -> Result<Self, String> {
        let ruleset: Self =
            serde_json::from_str(json).map_err(|e| format!("Invalid ruleset: {e}"))?;
        if ruleset.version != 1
            || ruleset.name.trim().is_empty()
            || ruleset.name.len() > 80
            || ruleset.summary.trim().is_empty()
            || ruleset.summary.len() > 4000
        {
            return Err("Unsupported ruleset version, title, or summary.".into());
        }
        match &ruleset.rules {
            Rules::Solitaire(rules) => rules.validate()?,
            Rules::Trick(rules) => rules.validate()?,
            Rules::Poker(rules) => rules.validate()?,
            Rules::Blackjack(rules) => rules.validate()?,
            Rules::Rummy(rules) => rules.validate()?,
            Rules::Partnership(rules) => rules.validate()?,
        }
        Ok(ruleset)
    }

    fn info(&self) -> wire::GameInfo {
        wire::GameInfo {
            name: self.name.clone(),
            rules: self.summary.clone(),
            artwork: match self.artwork {
                Artwork::Poker => "poker",
                Artwork::Bridge => "bridge",
            }
            .into(),
            multiplayer: !matches!(self.rules, Rules::Solitaire(_)),
        }
    }
}

struct Choice<T> {
    op: T,
    view: wire::Action,
}

impl<T> Choice<T> {
    fn new(op: T, label: String, source: u32, card: u32, destination: u32) -> Self {
        Self {
            op,
            view: wire::Action {
                id: 0,
                label,
                source,
                card,
                destination,
                selection: None,
            },
        }
    }
}

enum Game {
    Solitaire(solitaire::Game, Vec<solitaire::Game>),
    Trick(Box<trick::Game>),
    Poker(poker::Game),
    Blackjack(blackjack::Game),
    Rummy(Box<rummy::Game>),
    Partnership(Box<partnership::Game>),
}

impl Game {
    fn actor_is_agent(&self) -> bool {
        match self {
            Self::Solitaire(..) | Self::Blackjack(_) => false,
            Self::Trick(game) => game.actor() > 0,
            Self::Poker(game) => game.actor() > 0,
            Self::Rummy(game) => game.actor() > 0,
            Self::Partnership(game) => game.actor() > 0,
        }
    }

    fn view(&self, revision: u64, agent: bool) -> wire::Table {
        fn to_wire<T>(choices: Vec<Choice<T>>) -> Vec<wire::Action> {
            choices
                .into_iter()
                .enumerate()
                .map(|(index, choice)| wire::Action {
                    id: index as u32 + 1,
                    ..choice.view
                })
                .collect()
        }
        let can_act = self.actor_is_agent() == agent;
        let mut table = match self {
            Self::Solitaire(game, undo) => {
                let mut table = game.table();
                table.can_undo = !undo.is_empty();
                if can_act {
                    table.actions = to_wire(game.choices());
                    if !table.finished && table.actions.is_empty() {
                        table.status.push_str(" · No legal moves");
                    }
                }
                table
            }
            Self::Trick(game) => {
                let mut table = game.table(agent);
                if can_act {
                    table.actions = to_wire(game.choices());
                }
                table
            }
            Self::Poker(game) => {
                let mut table = game.table(agent);
                if can_act {
                    table.actions = to_wire(game.choices());
                }
                table
            }
            Self::Blackjack(game) => {
                let mut table = game.table();
                if can_act {
                    table.actions = to_wire(game.choices());
                }
                table
            }
            Self::Rummy(game) => {
                let mut table = game.table(agent);
                if can_act {
                    table.actions = to_wire(game.choices());
                }
                table
            }
            Self::Partnership(game) => {
                let mut table = game.table(agent);
                if can_act {
                    table.actions = to_wire(game.choices());
                }
                table
            }
        };
        table.revision = revision;
        table
    }

    fn apply(&mut self, index: usize) -> Result<(), String> {
        let illegal = || "That move is unavailable.".to_string();
        match self {
            Self::Solitaire(game, undo) => {
                let choice = game.choices().into_iter().nth(index).ok_or_else(illegal)?;
                undo.push(game.clone());
                game.apply(choice.op);
            }
            Self::Trick(game) => {
                let choice = game.choices().into_iter().nth(index).ok_or_else(illegal)?;
                game.apply(choice.op);
            }
            Self::Poker(game) => {
                let choice = game.choices().into_iter().nth(index).ok_or_else(illegal)?;
                game.apply(choice.op);
            }
            Self::Blackjack(game) => {
                let choice = game.choices().into_iter().nth(index).ok_or_else(illegal)?;
                game.apply(choice.op);
            }
            Self::Rummy(game) => {
                let choice = game.choices().into_iter().nth(index).ok_or_else(illegal)?;
                game.apply(choice.op);
            }
            Self::Partnership(game) => {
                let choice = game.choices().into_iter().nth(index).ok_or_else(illegal)?;
                game.apply(choice.op);
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct Engine {
    session: Option<(wire::GameInfo, Game)>,
    revision: u64,
}

impl Engine {
    fn call(&mut self, request: wire::Request) -> wire::Reply {
        let id = request.id;
        match self.command(request.command) {
            Ok(reply) => wire::Reply { id, ..reply },
            Err(error) => wire::Reply {
                id,
                error,
                ..Default::default()
            },
        }
    }

    fn command(&mut self, command: Option<wire::request::Command>) -> Result<wire::Reply, String> {
        use wire::request::Command;
        let table = match command.ok_or("Missing command.")? {
            Command::Inspect(request) => {
                let info = Ruleset::read(&request.ruleset)?.info();
                return Ok(wire::Reply {
                    game: Some(info),
                    ..Default::default()
                });
            }
            Command::Start(request) => {
                let ruleset = Ruleset::read(&request.ruleset)?;
                let info = ruleset.info();
                let game = match ruleset.rules {
                    Rules::Solitaire(rules) => {
                        Game::Solitaire(solitaire::Game::new(rules, request.seed), Vec::new())
                    }
                    Rules::Trick(rules) => {
                        Game::Trick(Box::new(trick::Game::new(rules, request.seed)))
                    }
                    Rules::Poker(rules) => Game::Poker(poker::Game::new(rules, request.seed)),
                    Rules::Blackjack(rules) => {
                        Game::Blackjack(blackjack::Game::new(rules, request.seed))
                    }
                    Rules::Rummy(rules) => {
                        Game::Rummy(Box::new(rummy::Game::new(rules, request.seed)))
                    }
                    Rules::Partnership(rules) => {
                        Game::Partnership(Box::new(partnership::Game::new(rules, request.seed)))
                    }
                };
                self.revision += 1;
                let table = game.view(self.revision, false);
                self.session = Some((info.clone(), game));
                return Ok(wire::Reply {
                    game: Some(info),
                    table: Some(table),
                    ..Default::default()
                });
            }
            Command::View(request) => {
                let (_, game) = self.session.as_ref().ok_or("Start a game first.")?;
                if request.agent && !game.actor_is_agent() {
                    return Err("It is your turn.".into());
                }
                game.view(self.revision, request.agent)
            }
            Command::Apply(request) => {
                let (_, game) = self.session.as_mut().ok_or("Start a game first.")?;
                if request.revision != self.revision
                    || game.actor_is_agent() != request.agent
                    || request.action == 0
                {
                    return Err("That move no longer applies to this turn.".into());
                }
                game.apply(request.action as usize - 1)?;
                self.revision += 1;
                game.view(self.revision, false)
            }
            Command::Undo(_) => {
                let (_, state) = self.session.as_mut().ok_or("Start a game first.")?;
                let Game::Solitaire(game, undo) = state else {
                    return Err("Undo is available in solitaire.".into());
                };
                *game = undo.pop().ok_or("There is no move to undo.")?;
                self.revision += 1;
                state.view(self.revision, false)
            }
        };
        Ok(wire::Reply {
            table: Some(table),
            ..Default::default()
        })
    }
}

// ABI 1. Calls on a context must be serialized by its owner. Requests are copied
// during the call and result bytes are borrowed until the result is freed.
#[unsafe(no_mangle)]
pub extern "C" fn oj_abi_version() -> u32 {
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn oj_create() -> *mut c_void {
    Box::into_raw(Box::new(Engine::default())).cast()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn oj_destroy(context: *mut c_void) {
    if !context.is_null() {
        drop(unsafe { Box::from_raw(context.cast::<Engine>()) });
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn oj_call(
    context: *mut c_void,
    data: *const u8,
    len: u32,
    result: *mut *mut c_void,
) -> u32 {
    if result.is_null() {
        return 1;
    }
    unsafe {
        result.write(ptr::null_mut());
    }
    if context.is_null() || data.is_null() || len == 0 || len > 128 * 1024 {
        return 1;
    }
    let input = unsafe { slice::from_raw_parts(data, len as usize) };
    let Ok(request) = wire::Request::decode(input) else {
        return 1;
    };
    match catch_unwind(AssertUnwindSafe(|| {
        let engine = unsafe { &mut *context.cast::<Engine>() };
        engine.call(request).encode_to_vec()
    })) {
        Ok(bytes) => {
            unsafe {
                result.write(Box::into_raw(Box::new(bytes)).cast());
            }
            0
        }
        Err(_) => 2,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn oj_result_bytes(result: *const c_void, length: *mut u32) -> *const u8 {
    if result.is_null() || length.is_null() {
        return ptr::null();
    }
    let bytes = unsafe { &*result.cast::<Vec<u8>>() };
    unsafe {
        length.write(bytes.len() as u32);
    }
    bytes.as_ptr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn oj_result_free(result: *mut c_void) {
    if !result.is_null() {
        drop(unsafe { Box::from_raw(result.cast::<Vec<u8>>()) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wire::request::Command;

    #[test]
    fn bundled_rulesets_are_playable_and_filter_hidden_cards() {
        for file in std::fs::read_dir("../rulesets").unwrap() {
            let file = file.unwrap().path();
            if file.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let ruleset = std::fs::read_to_string(&file).unwrap();
            for seed in 0..8 {
                let mut engine = Engine::default();
                let first = engine
                    .command(Some(Command::Start(wire::Start {
                        ruleset: ruleset.clone(),
                        seed,
                    })))
                    .unwrap_or_else(|error| panic!("{}: {error}", file.display()));
                let mut table = first.table.unwrap();
                for i in 0..190 {
                    for zone in &table.zones {
                        for card in zone.cards.iter().filter(|c| c.hidden) {
                            assert!(card.id == 0 && card.suit.is_empty() && card.rank.is_empty());
                        }
                    }
                    let agent = table.agent_turn;
                    if agent {
                        assert!(table.actions.is_empty());
                        table = engine
                            .command(Some(Command::View(wire::View { agent })))
                            .unwrap()
                            .table
                            .unwrap();
                        assert!(!table.actions.is_empty());
                    }
                    if table.actions.is_empty() {
                        break;
                    }
                    let action = table.actions[(i * 13 + seed as usize) % table.actions.len()].id;
                    assert!(
                        engine
                            .command(Some(Command::Apply(wire::Apply {
                                action,
                                agent,
                                revision: 0
                            })))
                            .is_err()
                    );
                    table = engine
                        .command(Some(Command::Apply(wire::Apply {
                            action,
                            agent,
                            revision: table.revision,
                        })))
                        .unwrap_or_else(|error| panic!("{}: {error}", file.display()))
                        .table
                        .unwrap();
                }
            }
        }
    }

    #[test]
    fn blackjack_start_and_restart_deal_the_opening_hand() {
        let mut engine = Engine::default();
        let start = wire::Start {
            ruleset: include_str!("../../rulesets/blackjack.json").into(),
            seed: 1,
        };
        let first = engine
            .command(Some(Command::Start(start.clone())))
            .unwrap()
            .table
            .unwrap();
        assert_eq!(first.zones.len(), 2);
        assert_eq!(first.zones[0].cards.len(), 2);
        assert!(!first.zones[0].cards[0].hidden);
        assert!(first.zones[0].cards[1].hidden);
        assert_eq!(first.zones[1].cards.len(), 2);
        assert!(first.zones[1].cards.iter().all(|card| !card.hidden));
        assert_eq!(first.status, "490 play chips");
        assert!(!first.agent_turn);
        assert!(first.actions.iter().any(|action| action.label == "Hit"));
        let stand = first
            .actions
            .iter()
            .find(|action| action.label == "Stand")
            .unwrap();
        let settled = engine
            .command(Some(Command::Apply(wire::Apply {
                action: stand.id,
                revision: first.revision,
                agent: false,
            })))
            .unwrap()
            .table
            .unwrap();
        assert_eq!(settled.actions[0].label, "Deal next hand · 10 play chips");
        let mut restarted = engine
            .command(Some(Command::Start(start)))
            .unwrap()
            .table
            .unwrap();
        assert!(restarted.revision > settled.revision);
        restarted.revision = first.revision;
        assert_eq!(restarted, first);
    }

    #[test]
    fn undo_restores_the_previous_deal_position() {
        let mut engine = Engine::default();
        let ruleset = include_str!("../../rulesets/klondike.json").into();
        let start = engine
            .command(Some(Command::Start(wire::Start { ruleset, seed: 91 })))
            .unwrap()
            .table
            .unwrap();
        let draw = start
            .actions
            .iter()
            .find(|a| a.label == "Draw from stock")
            .unwrap()
            .id;
        let after = engine
            .command(Some(Command::Apply(wire::Apply {
                revision: start.revision,
                action: draw,
                agent: false,
            })))
            .unwrap()
            .table
            .unwrap();
        assert!(after.can_undo && after.zones != start.zones);
        let restored = engine
            .command(Some(Command::Undo(wire::Undo {})))
            .unwrap()
            .table
            .unwrap();
        assert_eq!(start.zones, restored.zones);
        assert!(!restored.can_undo);
    }
}
