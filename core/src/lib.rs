mod blackjack;
mod card;
mod cribbage;
mod partnership;
mod poker;
mod rummy;
mod solitaire;
mod tableau;
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

const MAX_REQUEST_BYTES: usize = 128 * 1024;
// A saved game must fit inside a restore request, including its envelope.
const MAX_SAVED_GAME_BYTES: usize = MAX_REQUEST_BYTES - 16;
// Replay depends on the seeded shuffle, game logic and legal action ordering.
const SAVE_VERSION: u32 = 1;

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
    Tableau(tableau::Rules),
    Cribbage(cribbage::Rules),
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
            Rules::Tableau(_) => {}
            Rules::Cribbage(rules) => rules.validate()?,
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
            multiplayer: !matches!(self.rules, Rules::Solitaire(_) | Rules::Tableau(_)),
            hints: match &self.rules {
                Rules::Solitaire(_) | Rules::Tableau(_) => true,
                Rules::Trick(rules) => rules.advised(),
                Rules::Rummy(rules) => rules.advised(),
                Rules::Partnership(rules) => rules.advised(),
                _ => false,
            },
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
    Tableau(tableau::Game, Vec<tableau::Game>),
    Cribbage(Box<cribbage::Game>),
    Trick(Box<trick::Game>),
    Poker(poker::Game),
    Blackjack(blackjack::Game),
    Rummy(Box<rummy::Game>),
    Partnership(Box<partnership::Game>),
}

impl Game {
    fn actor_is_agent(&self) -> bool {
        match self {
            Self::Solitaire(..) | Self::Tableau(..) | Self::Blackjack(_) => false,
            Self::Cribbage(game) => game.actor() > 0,
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
        fn solitaire_table<T>(
            mut table: wire::Table,
            choices: Vec<Choice<T>>,
            can_undo: bool,
            priority: impl Fn(&T) -> u8,
        ) -> wire::Table {
            table.can_undo = can_undo;
            table.hint_action = choices
                .iter()
                .enumerate()
                .min_by_key(|(_, choice)| priority(&choice.op))
                .map_or(0, |(index, _)| index as u32 + 1);
            table.actions = to_wire(choices);
            if !table.finished && table.actions.is_empty() {
                table.status.push_str(" · No legal moves");
            }
            table
        }
        // The same advice plays the other seats and serves as the person's hint.
        fn advised_table<T: PartialEq>(
            mut table: wire::Table,
            choices: Vec<Choice<T>>,
            can_act: bool,
            agent: bool,
            advise: impl FnOnce() -> Option<T>,
        ) -> wire::Table {
            if !can_act {
                return table;
            }
            let advised = advise()
                .and_then(|advice| choices.iter().position(|c| c.op == advice))
                .map_or(0, |index| index as u32 + 1);
            if agent {
                table.agent_action = advised;
            } else {
                table.hint_action = advised;
            }
            table.actions = to_wire(choices);
            table
        }
        let can_act = self.actor_is_agent() == agent;
        let mut table = match self {
            Self::Solitaire(game, undo) => {
                solitaire_table(game.table(), game.choices(), !undo.is_empty(), |action| {
                    game.hint_priority(*action)
                })
            }
            Self::Tableau(game, undo) => {
                solitaire_table(game.table(), game.choices(), !undo.is_empty(), |action| {
                    tableau::Game::hint_priority(*action)
                })
            }
            Self::Cribbage(game) => {
                let mut table = game.table(agent);
                if can_act {
                    table.actions = to_wire(game.choices());
                }
                table
            }
            Self::Trick(game) => {
                let table = game.table(agent);
                advised_table(table, game.choices(), can_act, agent, || game.advise())
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
                let table = game.table(agent);
                advised_table(table, game.choices(), can_act, agent, || game.advise())
            }
            Self::Partnership(game) => {
                let table = game.table(agent);
                advised_table(table, game.choices(), can_act, agent, || game.advise())
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
            Self::Tableau(game, undo) => {
                let choice = game.choices().into_iter().nth(index).ok_or_else(illegal)?;
                undo.push(game.clone());
                game.apply(choice.op);
            }
            Self::Cribbage(game) => {
                let choice = game.choices().into_iter().nth(index).ok_or_else(illegal)?;
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

    fn undo(&mut self) -> Result<(), String> {
        let empty = "There is no move to undo.";
        match self {
            Self::Solitaire(game, undo) => *game = undo.pop().ok_or(empty)?,
            Self::Tableau(game, undo) => *game = undo.pop().ok_or(empty)?,
            _ => return Err("Undo is available in solitaire.".into()),
        }
        Ok(())
    }
}

struct Session {
    info: wire::GameInfo,
    game: Game,
    save: wire::SavedGame,
}

impl Session {
    fn start(start: wire::Start) -> Result<Self, String> {
        let ruleset = Ruleset::read(&start.ruleset)?;
        let info = ruleset.info();
        let seed = start.seed;
        let game = match ruleset.rules {
            Rules::Solitaire(rules) => {
                Game::Solitaire(solitaire::Game::new(rules, seed), Vec::new())
            }
            Rules::Tableau(rules) => Game::Tableau(tableau::Game::new(rules, seed), Vec::new()),
            Rules::Cribbage(rules) => Game::Cribbage(Box::new(cribbage::Game::new(rules, seed))),
            Rules::Trick(rules) => Game::Trick(Box::new(trick::Game::new(rules, seed))),
            Rules::Poker(rules) => Game::Poker(poker::Game::new(rules, seed)),
            Rules::Blackjack(rules) => Game::Blackjack(blackjack::Game::new(rules, seed)),
            Rules::Rummy(rules) => Game::Rummy(Box::new(rummy::Game::new(rules, seed))),
            Rules::Partnership(rules) => {
                Game::Partnership(Box::new(partnership::Game::new(rules, seed)))
            }
        };
        let save = wire::SavedGame {
            version: SAVE_VERSION,
            start: Some(start),
            actions: Vec::new(),
        };
        Ok(Self { info, game, save })
    }

    fn restore(save: wire::SavedGame) -> Result<Self, String> {
        if save.version != SAVE_VERSION {
            return Err("This saved game uses an unsupported format.".into());
        }
        if save.encoded_len() > MAX_SAVED_GAME_BYTES {
            return Err("This saved game is too large to restore.".into());
        }
        let mut session = Self::start(save.start.ok_or("The saved game is missing its deal.")?)?;
        for action in save.actions {
            session.apply(action).map_err(|error| {
                format!(
                    "Saved move {} could not be replayed. {error}",
                    session.save.actions.len() + 1
                )
            })?;
        }
        Ok(session)
    }

    fn apply(&mut self, action: u32) -> Result<(), String> {
        let index = action.checked_sub(1).ok_or("That move is unavailable.")?;
        self.game.apply(index as usize)?;
        self.save.actions.push(action);
        Ok(())
    }
}

#[derive(Default)]
struct Engine {
    session: Option<Session>,
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
            Command::Start(request) => return Ok(self.open(Session::start(request)?)),
            Command::Restore(save) => return Ok(self.open(Session::restore(save)?)),
            Command::Checkpoint(_) => {
                let save = &self.session.as_ref().ok_or("Start a game first.")?.save;
                if save.encoded_len() > MAX_SAVED_GAME_BYTES {
                    return Err("This game has too many moves to save.".into());
                }
                return Ok(wire::Reply {
                    saved_game: Some(save.clone()),
                    ..Default::default()
                });
            }
            Command::View(request) => {
                let game = &self.session.as_ref().ok_or("Start a game first.")?.game;
                if request.agent && !game.actor_is_agent() {
                    return Err("It is your turn.".into());
                }
                game.view(self.revision, request.agent)
            }
            Command::Apply(request) => {
                let session = self.session.as_mut().ok_or("Start a game first.")?;
                if request.revision != self.revision
                    || session.game.actor_is_agent() != request.agent
                    || request.action == 0
                {
                    return Err("That move no longer applies to this turn.".into());
                }
                session.apply(request.action)?;
                self.revision += 1;
                session.game.view(self.revision, false)
            }
            Command::Undo(_) => {
                let session = self.session.as_mut().ok_or("Start a game first.")?;
                session.game.undo()?;
                session.save.actions.pop();
                self.revision += 1;
                session.game.view(self.revision, false)
            }
        };
        Ok(wire::Reply {
            table: Some(table),
            ..Default::default()
        })
    }

    fn open(&mut self, session: Session) -> wire::Reply {
        self.revision += 1;
        let table = session.game.view(self.revision, false);
        let info = session.info.clone();
        self.session = Some(session);
        wire::Reply {
            game: Some(info),
            table: Some(table),
            ..Default::default()
        }
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
    if context.is_null() || data.is_null() || len == 0 || len as usize > MAX_REQUEST_BYTES {
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
    fn numbered_deals_keep_their_opening_layouts() {
        let cases: [(&str, &[u32]); 3] = [
            (
                include_str!("../../rulesets/klondike.json"),
                &[22, 17, 30, 51, 46, 52, 47],
            ),
            (
                include_str!("../../rulesets/double-klondike.json"),
                &[100, 25, 42, 29, 76, 18, 86, 19, 9],
            ),
            (
                include_str!("../../rulesets/tripeaks.json"),
                &[12, 51, 7, 13, 39, 46, 34, 43, 52, 16, 47],
            ),
        ];
        let mut engine = Engine::default();
        for (ruleset, expected) in cases {
            let table = engine
                .command(Some(Command::Start(wire::Start {
                    ruleset: ruleset.into(),
                    seed: 42,
                })))
                .unwrap()
                .table
                .unwrap();
            let visible: Vec<_> = table
                .zones
                .iter()
                .flat_map(|z| &z.cards)
                .filter(|c| !c.hidden)
                .map(|c| c.id)
                .collect();
            assert_eq!(visible, expected);
        }
    }

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
                let info = first.game.unwrap();
                let multiplayer = info.multiplayer;
                let built_in = ["euchre", "hearts", "spades", "rummy", "gin-rummy"]
                    .contains(&file.file_stem().unwrap().to_str().unwrap());
                assert_eq!(info.hints, !multiplayer || built_in, "{}", file.display());
                let mut table = first.table.unwrap();
                for i in 0..190 {
                    for zone in &table.zones {
                        for card in zone.cards.iter().filter(|c| c.hidden) {
                            assert!(card.id == 0 && card.suit.is_empty() && card.rank.is_empty());
                        }
                    }
                    let agent = table.agent_turn;
                    assert_eq!(table.agent_action, 0);
                    if agent {
                        assert!(table.actions.is_empty());
                        table = engine
                            .command(Some(Command::View(wire::View { agent })))
                            .unwrap()
                            .table
                            .unwrap();
                        assert!(!table.actions.is_empty());
                        assert_eq!(table.hint_action, 0);
                        assert_eq!(table.agent_action != 0, built_in, "{}", file.display());
                        if built_in {
                            assert!(table.actions.iter().any(|a| a.id == table.agent_action));
                        }
                    }
                    if agent || (multiplayer && !built_in) || table.actions.is_empty() {
                        assert_eq!(table.hint_action, 0);
                    } else if multiplayer && table.actions.len() == 1 {
                        // "Next deal" needs no suggestion.
                        assert!(table.hint_action <= 1);
                    } else {
                        let hint = table
                            .actions
                            .iter()
                            .find(|a| a.id == table.hint_action)
                            .unwrap();
                        // Bidding choices such as Pass have no source card.
                        if hint.card != 0 || !multiplayer {
                            let source = table.zones.iter().find(|z| z.id == hint.source).unwrap();
                            assert!(
                                hint.card == 0
                                    || source.cards.iter().any(|c| !c.hidden && c.id == hint.card)
                            );
                        }
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
    fn tripeaks_undo_restores_positions_and_hidden_cards() {
        let mut engine = Engine::default();
        let first = engine
            .command(Some(Command::Start(wire::Start {
                ruleset: include_str!("../../rulesets/tripeaks.json").into(),
                seed: 1,
            })))
            .unwrap();
        assert!(!first.game.unwrap().multiplayer);
        let table = first.table.unwrap();
        assert_eq!(
            table
                .zones
                .iter()
                .filter(|zone| zone.position.is_some())
                .count(),
            28
        );
        assert_eq!(table.actions[0].label, "Draw from stock");
        let drawn = engine
            .command(Some(Command::Apply(wire::Apply {
                revision: table.revision,
                action: table.actions[0].id,
                agent: false,
            })))
            .unwrap()
            .table
            .unwrap();
        assert!(drawn.can_undo);
        assert_ne!(drawn.zones, table.zones);
        let mut undone = engine
            .command(Some(Command::Undo(wire::Undo {})))
            .unwrap()
            .table
            .unwrap();
        undone.revision = table.revision;
        assert_eq!(undone, table);
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

    #[test]
    fn saved_games_restore_the_table_and_continue_across_all_rulesets() {
        for file in std::fs::read_dir("../rulesets").unwrap() {
            let file = file.unwrap().path();
            if file.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let mut live = Engine::default();
            let started = live
                .command(Some(Command::Start(wire::Start {
                    ruleset: std::fs::read_to_string(&file).unwrap(),
                    seed: 91,
                })))
                .unwrap();
            let mut table = started.table.unwrap();
            for step in 0..40 {
                let agent = table.agent_turn;
                let view = if agent {
                    live.command(Some(Command::View(wire::View { agent })))
                        .unwrap()
                        .table
                        .unwrap()
                } else {
                    table.clone()
                };
                let save = live
                    .command(Some(Command::Checkpoint(wire::Checkpoint {})))
                    .unwrap()
                    .saved_game
                    .unwrap();
                assert_eq!(save.actions.len(), step);
                let save = wire::SavedGame::decode(save.encode_to_vec().as_slice()).unwrap();
                let mut reopened = Engine::default();
                let restored = reopened.command(Some(Command::Restore(save))).unwrap();
                assert_eq!(restored.game, started.game);
                let mut restored_table = restored.table.unwrap();
                let restored_revision = restored_table.revision;
                restored_table.revision = table.revision;
                assert_eq!(restored_table, table, "{}", file.display());
                let Some(action) = view.actions.last().map(|action| action.id) else {
                    break;
                };
                let play = |engine: &mut Engine, revision| {
                    let mut next = engine
                        .command(Some(Command::Apply(wire::Apply {
                            action,
                            revision,
                            agent,
                        })))
                        .unwrap()
                        .table
                        .unwrap();
                    next.revision = table.revision + 1;
                    next
                };
                let next = play(&mut live, table.revision);
                assert_eq!(
                    play(&mut reopened, restored_revision),
                    next,
                    "{}",
                    file.display()
                );
                table = next;
            }
        }
    }

    #[test]
    fn undo_removes_saved_moves_and_invalid_restore_keeps_the_live_table() {
        let mut engine = Engine::default();
        let initial = engine
            .command(Some(Command::Start(wire::Start {
                ruleset: include_str!("../../rulesets/klondike.json").into(),
                seed: 91,
            })))
            .unwrap()
            .table
            .unwrap();
        let draw = initial
            .actions
            .iter()
            .find(|a| a.label == "Draw from stock")
            .unwrap()
            .id;
        let after = engine
            .command(Some(Command::Apply(wire::Apply {
                action: draw,
                agent: false,
                revision: initial.revision,
            })))
            .unwrap()
            .table
            .unwrap();
        let checkpoint = |engine: &mut Engine| {
            engine
                .command(Some(Command::Checkpoint(wire::Checkpoint {})))
                .unwrap()
                .saved_game
                .unwrap()
        };
        let save = checkpoint(&mut engine);
        let mut invalid = save.clone();
        invalid.actions.push(u32::MAX);
        assert!(engine.command(Some(Command::Restore(invalid))).is_err());
        let mut unsupported = save.clone();
        unsupported.version += 1;
        assert!(engine.command(Some(Command::Restore(unsupported))).is_err());
        assert_eq!(checkpoint(&mut engine), save);
        assert_eq!(engine.revision, after.revision);

        let mut restarted = Engine::default();
        restarted.command(Some(Command::Restore(save))).unwrap();
        let mut undone = restarted
            .command(Some(Command::Undo(wire::Undo {})))
            .unwrap()
            .table
            .unwrap();
        undone.revision = initial.revision;
        assert_eq!(undone, initial);
        assert!(checkpoint(&mut restarted).actions.is_empty());
    }
}
