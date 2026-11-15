use crate::{
    Choice,
    card::{self, Card, Suit},
    wire,
};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    stack: u32,
    small_blind: u32,
    max_wagers: u32,
}

impl Rules {
    pub fn validate(&self) -> Result<(), String> {
        if (1..=1000).contains(&self.small_blind)
            && (self.small_blind * 20..=100_000).contains(&self.stack)
            && (2..=8).contains(&self.max_wagers)
        {
            Ok(())
        } else {
            Err("Invalid Hold’em stack, blind, or betting limit.".into())
        }
    }
}

#[derive(Clone, Copy)]
pub enum Move {
    Fold,
    Match,
    Raise,
    Next,
}

const NAMES: [&str; 2] = ["You", "Old James"];
const STREET: [&str; 4] = ["Preflop", "Flop", "Turn", "River"];

pub struct Game {
    rules: Rules,
    seed: u64,
    hand: u32,
    button: usize,
    turn: usize,
    street: usize,
    stock: Vec<Card>,
    hands: [Vec<Card>; 2],
    board: Vec<Card>,
    chips: [u32; 2],
    stake: [u32; 2],
    pot: u32,
    wagers: u32,
    pending: [bool; 2],
    settled: bool,
    reveal: bool,
    result: String,
    events: Vec<String>,
}

impl Game {
    pub fn new(rules: Rules, seed: u64) -> Self {
        let stack = rules.stack;
        let mut game = Self {
            rules,
            seed,
            hand: 0,
            button: 0,
            turn: 0,
            street: 0,
            stock: vec![],
            hands: Default::default(),
            board: vec![],
            chips: [stack; 2],
            stake: [0; 2],
            pot: 0,
            wagers: 0,
            pending: [false; 2],
            settled: false,
            reveal: false,
            result: String::new(),
            events: vec![],
        };
        game.deal();
        game
    }

    pub fn actor(&self) -> usize {
        if self.settled { 0 } else { self.turn }
    }

    fn deal(&mut self) {
        self.button = self.hand as usize % 2;
        self.turn = self.button;
        self.hand += 1;
        self.street = 0;
        self.stock = card::deck(&Suit::ALL, 1, self.seed.wrapping_add(u64::from(self.hand)));
        self.hands = Default::default();
        for _ in 0..2 {
            for seat in [1 - self.button, self.button] {
                self.hands[seat].push(self.stock.pop().unwrap());
            }
        }
        self.board.clear();
        self.events.clear();
        self.pot = 0;
        self.stake = [0; 2];
        self.pending = [true; 2];
        self.settled = false;
        self.reveal = false;
        let effective = self.chips[0].min(self.chips[1]);
        let small = effective.min(self.rules.small_blind);
        let big = effective.min(self.rules.small_blind * 2);
        self.pay(self.button, small);
        self.pay(1 - self.button, big);
        self.wagers = 1;
        self.events.push(format!(
            "{} posts {small}; {} posts {big}.",
            NAMES[self.button],
            NAMES[1 - self.button]
        ));
        if effective == small {
            self.run_out();
        }
    }

    fn pay(&mut self, seat: usize, chips: u32) {
        self.chips[seat] -= chips;
        self.stake[seat] += chips;
        self.pot += chips;
    }

    fn raise_to(&self) -> Option<u32> {
        let high = self.stake[0].max(self.stake[1]);
        let cap = (self.stake[0] + self.chips[0]).min(self.stake[1] + self.chips[1]);
        let size = self.rules.small_blind * if self.street < 2 { 2 } else { 4 };
        if cap <= high || self.wagers >= self.rules.max_wagers {
            return None;
        }
        Some((high + size).min(cap))
    }

    pub fn choices(&self) -> Vec<Choice<Move>> {
        let make = |op, label| Choice::new(op, label, 0, 0, 0);
        if self.settled {
            return if self.chips.contains(&0) {
                vec![]
            } else {
                vec![make(Move::Next, "Next hand".into())]
            };
        }
        let owed = self.stake[1 - self.turn].saturating_sub(self.stake[self.turn]);
        let mut moves = vec![make(
            Move::Match,
            if owed == 0 {
                "Check".into()
            } else {
                format!("Call {owed}")
            },
        )];
        if owed > 0 {
            moves.insert(0, make(Move::Fold, "Fold".into()));
        }
        if let Some(to) = self.raise_to() {
            moves.push(make(
                Move::Raise,
                if self.stake == [0, 0] {
                    format!("Bet {to}")
                } else {
                    format!("Raise to {to}")
                },
            ));
        }
        moves
    }

    pub fn apply(&mut self, action: Move) {
        let seat = self.turn;
        let other = 1 - seat;
        match action {
            Move::Next => self.deal(),
            Move::Fold => {
                self.events.push(format!("{} folds", NAMES[seat]));
                self.award(Some(other), false);
            }
            Move::Match => {
                let owed = self.stake[other].saturating_sub(self.stake[seat]);
                self.events.push(if owed > 0 {
                    format!("{} calls {owed}", NAMES[seat])
                } else {
                    format!("{} checks", NAMES[seat])
                });
                self.pay(seat, owed);
                self.pending[seat] = false;
                if self.chips.contains(&0) && self.stake[0] == self.stake[1] {
                    self.run_out();
                } else if self.pending[other] {
                    self.turn = other;
                } else {
                    self.advance();
                }
            }
            Move::Raise => {
                let to = self.raise_to().unwrap();
                self.events.push(if self.stake == [0, 0] {
                    format!("{} bets {to}", NAMES[seat])
                } else {
                    format!("{} raises to {to}", NAMES[seat])
                });
                self.pay(seat, to - self.stake[seat]);
                self.wagers += 1;
                self.pending[seat] = false;
                self.pending[other] = true;
                self.turn = other;
            }
        }
    }

    fn advance(&mut self) {
        if self.street == 3 {
            self.showdown();
            return;
        }
        self.reveal_street();
        self.stake = [0; 2];
        self.wagers = 0;
        self.pending = [true; 2];
        self.turn = 1 - self.button;
    }

    fn reveal_street(&mut self) {
        self.stock.pop();
        self.street += 1;
        let count = if self.street == 1 { 3 } else { 1 };
        self.board
            .extend((0..count).map(|_| self.stock.pop().unwrap()));
        self.events.push(format!(
            "{}: {}",
            STREET[self.street],
            cards_text(&self.board)
        ));
    }

    fn run_out(&mut self) {
        while self.street < 3 {
            self.reveal_street();
        }
        self.showdown();
    }

    fn showdown(&mut self) {
        let ranks: [[u8; 6]; 2] = std::array::from_fn(|seat| {
            let cards: Vec<_> = self.hands[seat]
                .iter()
                .chain(&self.board)
                .copied()
                .collect();
            holdem_rank(&cards)
        });
        self.events.push(format!(
            "{}: {} ({})",
            NAMES[0],
            cards_text(&self.hands[0]),
            hand_name(ranks[0][0])
        ));
        self.events.push(format!(
            "{}: {} ({})",
            NAMES[1],
            cards_text(&self.hands[1]),
            hand_name(ranks[1][0])
        ));
        let winner = match ranks[0].cmp(&ranks[1]) {
            std::cmp::Ordering::Greater => Some(0),
            std::cmp::Ordering::Less => Some(1),
            _ => None,
        };
        self.award(winner, true);
    }

    fn award(&mut self, winner: Option<usize>, reveal: bool) {
        let chips = std::mem::take(&mut self.pot);
        self.stake = [0; 2];
        self.settled = true;
        self.reveal = reveal;
        self.result = if let Some(winner) = winner {
            self.chips[winner] += chips;
            if winner == 0 {
                format!("You win {chips} chips")
            } else {
                format!("Old James wins {chips} chips")
            }
        } else {
            self.chips[0] += chips / 2;
            self.chips[1] += chips / 2;
            self.chips[1 - self.button] += chips % 2;
            format!("Split pot: {chips} chips")
        };
        if self.chips.contains(&0) {
            self.result.push_str(" · Match over");
        }
    }

    pub fn table(&self, agent: bool) -> wire::Table {
        use wire::zone::Layout::Hand;
        let viewer = usize::from(agent);
        let opponent = 1 - viewer;
        let dealer = NAMES[self.button];
        let zones = vec![
            card::zone(
                1,
                0,
                format!("{} · {} chips", NAMES[opponent], self.chips[opponent]),
                Hand,
                if self.reveal {
                    self.hands[opponent].iter().map(|c| c.view()).collect()
                } else {
                    vec![card::hidden(); 2]
                },
                2,
            ),
            card::zone(
                2,
                1,
                format!("Board · Pot {}", self.pot),
                Hand,
                self.board.iter().map(|c| c.view()).collect(),
                5,
            ),
            card::zone(
                3,
                2,
                format!("{} · {} chips", NAMES[viewer], self.chips[viewer]),
                Hand,
                self.hands[viewer].iter().map(|c| c.view()).collect(),
                2,
            ),
        ];
        let status = if self.settled {
            self.result.clone()
        } else {
            format!("{} · {} to act", STREET[self.street], NAMES[self.turn])
        };
        let player_view = format!(
            "You are {}. Your cards: {}. Board: {}. Pot: {}. Button: {}. Remaining stacks: You (human) {}, Old James {}. Current contributions: human {}, Old James {}.",
            NAMES[viewer],
            cards_text(&self.hands[viewer]),
            cards_text(&self.board),
            self.pot,
            dealer,
            self.chips[0],
            self.chips[1],
            self.stake[0],
            self.stake[1]
        );
        wire::Table {
            zones,
            status,
            player_view,
            agent_turn: self.actor() == 1,
            finished: self.settled && self.chips.contains(&0),
            events: self.events.clone(),
            ..Default::default()
        }
    }
}

fn cards_text(cards: &[Card]) -> String {
    cards
        .iter()
        .map(|c| c.short())
        .collect::<Vec<_>>()
        .join(" ")
}

fn hand_name(category: u8) -> &'static str {
    [
        "high card",
        "pair",
        "two pair",
        "three of a kind",
        "straight",
        "flush",
        "full house",
        "four of a kind",
        "straight flush",
    ][category as usize]
}

fn holdem_rank(cards: &[Card]) -> [u8; 6] {
    let mut best = [0; 6];
    for a in 0..cards.len() {
        for b in a + 1..cards.len() {
            let five = cards
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != a && *i != b)
                .map(|(_, c)| *c)
                .collect::<Vec<_>>();
            best = best.max(rank_five(&five));
        }
    }
    best
}

fn rank_five(cards: &[Card]) -> [u8; 6] {
    let mut ranks: Vec<_> = cards.iter().map(|c| c.high()).collect();
    ranks.sort_unstable_by(|a, b| b.cmp(a));
    let flush = cards.iter().all(|c| c.suit == cards[0].suit);
    let straight = if ranks == [14, 5, 4, 3, 2] {
        5
    } else if ranks.windows(2).all(|r| r[0] == r[1] + 1) {
        ranks[0]
    } else {
        0
    };
    let mut groups: Vec<_> = (2..=14)
        .rev()
        .map(|r| (ranks.iter().filter(|v| **v == r).count(), r))
        .filter(|&(n, _)| n != 0)
        .collect();
    groups.sort_unstable_by(|a, b| b.cmp(a));
    let category = match (groups[0].0, groups[1].0, flush, straight) {
        (_, _, true, 1..) => 8,
        (4, _, _, _) => 7,
        (3, 2, _, _) => 6,
        (_, _, true, _) => 5,
        (_, _, _, 1..) => 4,
        (3, _, _, _) => 3,
        (2, 2, _, _) => 2,
        (2, _, _, _) => 1,
        _ => 0,
    };
    let mut result = [0; 6];
    result[0] = category;
    if matches!(category, 4 | 8) {
        result[1] = straight;
    } else {
        for (i, &(_, rank)) in groups.iter().enumerate() {
            result[i + 1] = rank;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_wheel_and_prefers_best_five() {
        let cards: Vec<_> = [1, 2, 3, 4, 5, 12, 13]
            .into_iter()
            .map(|rank| Card {
                id: u32::from(rank),
                suit: Suit::Spades,
                rank,
            })
            .collect();
        assert_eq!(holdem_rank(&cards), [8, 5, 0, 0, 0, 0]);
    }
}
