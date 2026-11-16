use crate::{
    Choice,
    card::{self, Card, Suit},
    wire,
};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    #[serde(default)]
    variant: Variant,
    stack: u32,
    small_blind: u32,
    max_wagers: u32,
    #[serde(default)]
    ante: u32,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Variant {
    #[default]
    Holdem,
    Omaha,
    Draw,
    Stud,
}

impl Rules {
    pub fn validate(&self) -> Result<(), String> {
        if (1..=1000).contains(&self.small_blind)
            && (self.small_blind * 20..=100_000).contains(&self.stack)
            && (2..=8).contains(&self.max_wagers)
            && if self.variant == Variant::Stud {
                (1..=self.small_blind).contains(&self.ante)
            } else {
                self.ante == 0
            }
        {
            Ok(())
        } else {
            Err("Invalid poker stack, ante, blind, or betting limit.".into())
        }
    }
}

#[derive(Clone, Copy)]
pub enum Move {
    Fold,
    Match,
    Raise,
    Next,
    Draw(u8),
}

const NAMES: [&str; 2] = ["You", "Old James"];
const STREET: [&str; 4] = ["Preflop", "Flop", "Turn", "River"];
const STUD_STREET: [&str; 5] = [
    "Third street",
    "Fourth street",
    "Fifth street",
    "Sixth street",
    "Seventh street",
];
const HAND: u32 = 3;

impl Variant {
    fn initial_cards(self) -> usize {
        match self {
            Self::Holdem => 2,
            Self::Omaha => 4,
            Self::Draw => 5,
            Self::Stud => 3,
        }
    }

    fn last_street(self) -> usize {
        match self {
            Self::Holdem | Self::Omaha => 3,
            Self::Draw => 1,
            Self::Stud => 4,
        }
    }
}

pub struct Game {
    rules: Rules,
    seed: u64,
    hand: u32,
    button: usize,
    turn: usize,
    street: usize,
    drawing: bool,
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
            drawing: false,
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
        self.drawing = false;
        self.stock = card::deck(&Suit::ALL, 1, self.seed.wrapping_add(u64::from(self.hand)));
        self.hands = Default::default();
        for _ in 0..self.rules.variant.initial_cards() {
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
        self.wagers = 0;
        let effective = self.chips[0].min(self.chips[1]);
        if self.rules.variant == Variant::Stud {
            let ante = effective.min(self.rules.ante);
            self.chips.iter_mut().for_each(|stack| *stack -= ante);
            self.pot = ante * 2;
            self.events.push(format!("Both players ante {ante}."));
            self.turn = if bring_in_key(self.hands[0][2]) < bring_in_key(self.hands[1][2]) {
                0
            } else {
                1
            };
            if self.chips.contains(&0) {
                self.run_out();
            }
            return;
        }
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
            self.run_out_or_draw();
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
        if cap <= high || self.wagers >= self.rules.max_wagers {
            return None;
        }
        let size = self.bet_size();
        let next = if self.rules.variant == Variant::Stud && self.street == 0 && self.wagers == 0 {
            size
        } else {
            high + size
        };
        Some(next.min(cap))
    }

    fn bet_size(&self) -> u32 {
        let big_bet_street = if self.rules.variant == Variant::Draw {
            1
        } else {
            2
        };
        let multiplier = if self.street < big_bet_street { 2 } else { 4 };
        self.rules.small_blind * multiplier
    }

    fn must_bring_in(&self) -> bool {
        self.rules.variant == Variant::Stud && self.street == 0 && self.stake == [0, 0]
    }

    fn bring_in(&self) -> u32 {
        self.rules.small_blind.min(self.chips[0].min(self.chips[1]))
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
        if self.drawing {
            return (0..1 << self.hands[self.turn].len())
                .map(|mask| {
                    let cards = self.hands[self.turn]
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| mask & (1 << i) != 0)
                        .map(|(_, &card)| card)
                        .collect::<Vec<_>>();
                    let label = if cards.is_empty() {
                        "Stand pat".into()
                    } else {
                        format!("Draw {} · replace {}", cards.len(), cards_text(&cards))
                    };
                    let mut choice = make(Move::Draw(mask), label);
                    choice.view.source = HAND;
                    choice.view.selection = Some(wire::CardSelection {
                        cards: cards.iter().map(|card| card.id).collect(),
                    });
                    choice
                })
                .collect();
        }
        if self.must_bring_in() {
            let bring_in = self.bring_in();
            let mut moves = vec![make(Move::Match, format!("Bring in {bring_in}"))];
            if let Some(to) = self.raise_to().filter(|&to| to > bring_in) {
                moves.push(make(Move::Raise, format!("Complete to {to}")));
            }
            return moves;
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
                } else if self.rules.variant == Variant::Stud
                    && self.street == 0
                    && self.wagers == 0
                {
                    format!("Complete to {to}")
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
            Move::Draw(mask) => {
                let mut count = 0;
                for (i, card) in self.hands[seat].iter_mut().enumerate() {
                    if mask & (1 << i) != 0 {
                        *card = self.stock.pop().unwrap();
                        count += 1;
                    }
                }
                self.events.push(match (seat, count) {
                    (0, 0) => "You stand pat".into(),
                    (0, _) => format!("You draw {count}"),
                    (_, 0) => "Old James stands pat".into(),
                    _ => format!("Old James draws {count}"),
                });
                if seat == self.button {
                    self.drawing = false;
                    if self.chips.contains(&0) {
                        self.showdown();
                    } else {
                        self.start_street(1 - self.button);
                    }
                } else {
                    self.turn = other;
                }
            }
            Move::Fold => {
                self.events.push(format!("{} folds", NAMES[seat]));
                self.award(Some(other), false);
            }
            Move::Match => {
                if self.must_bring_in() {
                    let chips = self.bring_in();
                    self.pay(seat, chips);
                    self.events
                        .push(format!("{} brings in {chips}", NAMES[seat]));
                    self.pending[seat] = false;
                    self.turn = other;
                    return;
                }
                let owed = self.stake[other].saturating_sub(self.stake[seat]);
                self.events.push(if owed > 0 {
                    format!("{} calls {owed}", NAMES[seat])
                } else {
                    format!("{} checks", NAMES[seat])
                });
                self.pay(seat, owed);
                self.pending[seat] = false;
                if self.chips.contains(&0) && self.stake[0] == self.stake[1] {
                    self.run_out_or_draw();
                } else if self.pending[other] {
                    self.turn = other;
                } else {
                    self.advance();
                }
            }
            Move::Raise => {
                let to = self.raise_to().unwrap();
                self.events.push(
                    if self.rules.variant == Variant::Stud && self.street == 0 && self.wagers == 0 {
                        format!("{} completes to {to}", NAMES[seat])
                    } else if self.stake == [0, 0] {
                        format!("{} bets {to}", NAMES[seat])
                    } else {
                        format!("{} raises to {to}", NAMES[seat])
                    },
                );
                self.pay(seat, to - self.stake[seat]);
                self.wagers += 1;
                self.pending[seat] = false;
                self.pending[other] = true;
                self.turn = other;
            }
        }
    }

    fn advance(&mut self) {
        if self.street == self.rules.variant.last_street() {
            self.showdown();
            return;
        }
        if self.rules.variant == Variant::Draw {
            self.begin_draw();
            return;
        }
        self.reveal_street();
        let opener = if self.rules.variant == Variant::Stud {
            let left_of_button = 1 - self.button;
            let other = &self.hands[self.button];
            let left = &self.hands[left_of_button];
            if groups(stud_upcards(left)) >= groups(stud_upcards(other)) {
                left_of_button
            } else {
                self.button
            }
        } else {
            1 - self.button
        };
        self.start_street(opener);
    }

    fn start_street(&mut self, opener: usize) {
        self.stake = [0; 2];
        self.wagers = 0;
        self.pending = [true; 2];
        self.turn = opener;
    }

    fn begin_draw(&mut self) {
        self.street = 1;
        self.drawing = true;
        self.start_street(1 - self.button);
    }

    fn run_out_or_draw(&mut self) {
        if self.rules.variant == Variant::Draw && self.street == 0 {
            self.begin_draw();
        } else {
            self.run_out();
        }
    }

    fn reveal_street(&mut self) {
        self.stock.pop();
        self.street += 1;
        if self.rules.variant == Variant::Stud {
            for seat in [1 - self.button, self.button] {
                let card = self.stock.pop().unwrap();
                self.hands[seat].push(card);
            }
            self.events.push(if self.street == 4 {
                "Seventh street: one face-down card each".into()
            } else {
                format!(
                    "{}: {} {}, {} {}",
                    STUD_STREET[self.street],
                    NAMES[0],
                    self.hands[0].last().unwrap().short(),
                    NAMES[1],
                    self.hands[1].last().unwrap().short()
                )
            });
            return;
        }
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
        while self.street < self.rules.variant.last_street() {
            self.reveal_street();
        }
        self.showdown();
    }

    fn showdown(&mut self) {
        let ranks: [[u8; 6]; 2] = std::array::from_fn(|seat| {
            match self.rules.variant {
                Variant::Omaha => return omaha_rank(&self.hands[seat], &self.board),
                Variant::Draw => return rank_five(&self.hands[seat]),
                Variant::Stud => return rank_seven(&self.hands[seat]),
                Variant::Holdem => {}
            }
            let cards: Vec<_> = self.hands[seat]
                .iter()
                .chain(&self.board)
                .copied()
                .collect();
            rank_seven(&cards)
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
        let stud = self.rules.variant == Variant::Stud;
        let community = matches!(self.rules.variant, Variant::Holdem | Variant::Omaha);
        let hand = |seat: usize, id, row| {
            let private = self.hands[seat]
                .iter()
                .enumerate()
                .filter(|(i, _)| !stud || !is_stud_upcard(*i));
            let cards = private
                .map(|(_, card)| {
                    if viewer == seat || self.reveal {
                        card.view()
                    } else {
                        card::hidden()
                    }
                })
                .collect::<Vec<_>>();
            let count = cards.len();
            card::zone(
                id,
                row,
                format!("{} · {} chips", NAMES[seat], self.chips[seat]),
                Hand,
                cards,
                count,
            )
        };
        let exposed = |seat: usize, id, row| {
            let cards = stud_upcards(&self.hands[seat]);
            card::zone(
                id,
                row,
                "Face-up cards".into(),
                Hand,
                cards.iter().map(|c| c.view()).collect(),
                cards.len(),
            )
        };
        let mut zones = vec![hand(opponent, 1, 0)];
        if stud {
            zones.push(exposed(opponent, 4, 0));
        }
        if community {
            zones.push(card::zone(
                2,
                1,
                format!("Board · Pot {}", self.pot),
                Hand,
                self.board.iter().map(|c| c.view()).collect(),
                5,
            ));
        }
        zones.push(hand(viewer, HAND, if community { 2 } else { 1 }));
        if stud {
            zones.push(exposed(viewer, 5, 1));
        }
        let status = if self.settled {
            self.result.clone()
        } else {
            let phase = match self.rules.variant {
                Variant::Draw if self.drawing => "Draw",
                Variant::Draw if self.street == 0 => "Before the draw",
                Variant::Draw => "After the draw",
                Variant::Stud => STUD_STREET[self.street],
                _ => STREET[self.street],
            };
            let verb = if self.drawing { "to draw" } else { "to act" };
            let mut label = format!("{phase} · {} {verb}", NAMES[self.turn]);
            if !community {
                label.push_str(&format!(" · Pot {}", self.pot));
            }
            label
        };
        let shared = if stud {
            format!(
                "Your exposed cards: {}. Opponent’s exposed cards: {}. ",
                cards_text(stud_upcards(&self.hands[viewer])),
                cards_text(stud_upcards(&self.hands[opponent]))
            )
        } else if community {
            format!("Board: {}. ", cards_text(&self.board))
        } else {
            String::new()
        };
        let player_view = format!(
            "You are {}. Your cards: {}. {shared}Pot: {}. Button: {}. Remaining stacks: You (human) {}, Old James {}. Current contributions: human {}, Old James {}. {status}.",
            NAMES[viewer],
            cards_text(&self.hands[viewer]),
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

fn is_stud_upcard(index: usize) -> bool {
    (2..=5).contains(&index)
}

fn stud_upcards(hand: &[Card]) -> &[Card] {
    &hand[2..hand.len().min(6)]
}

fn bring_in_key(card: Card) -> (u8, u8) {
    (card.high(), card.suit as u8)
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

fn rank_seven(cards: &[Card]) -> [u8; 6] {
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

fn omaha_rank(hand: &[Card], board: &[Card]) -> [u8; 6] {
    let mut best = [0; 6];
    for a in 0..hand.len() {
        for b in a + 1..hand.len() {
            for c in 0..board.len() {
                for d in c + 1..board.len() {
                    for e in d + 1..board.len() {
                        best =
                            best.max(rank_five(&[hand[a], hand[b], board[c], board[d], board[e]]));
                    }
                }
            }
        }
    }
    best
}

fn groups(cards: &[Card]) -> Vec<(usize, u8)> {
    let mut counts = [0; 15];
    for card in cards {
        counts[usize::from(card.high())] += 1;
    }
    let mut groups: Vec<_> = (2..=14)
        .filter(|&r| counts[r] > 0)
        .map(|r| (counts[r], r as u8))
        .collect();
    groups.sort_unstable_by(|a, b| b.cmp(a));
    groups
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
    let groups = groups(cards);
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

    fn card(rank: u8, suit: Suit) -> Card {
        Card {
            id: u32::from(rank) + (suit as u32) * 13,
            suit,
            rank,
        }
    }

    fn game(variant: Variant) -> Game {
        Game::new(
            Rules {
                variant,
                stack: 500,
                small_blind: 5,
                ante: if variant == Variant::Stud { 1 } else { 0 },
                max_wagers: 4,
            },
            17,
        )
    }

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
        assert_eq!(rank_seven(&cards), [8, 5, 0, 0, 0, 0]);
    }

    #[test]
    fn omaha_uses_exactly_two_hole_cards_even_with_a_royal_flush_on_the_board() {
        let board = [10, 11, 12, 13, 1].map(|rank| card(rank, Suit::Spades));
        let mut hole = [2, 3, 4, 5].map(|rank| card(rank, Suit::Clubs));
        assert_eq!(omaha_rank(&hole, &board)[0], 0);
        hole[0] = card(8, Suit::Spades);
        assert_eq!(omaha_rank(&hole, &board)[0], 0);
        hole[1] = card(9, Suit::Spades);
        assert_eq!(omaha_rank(&hole, &board), [8, 12, 0, 0, 0, 0]);
    }

    #[test]
    fn draw_choices_replace_exactly_the_named_cards_even_when_all_in() {
        let mut game = game(Variant::Draw);
        game.chips[0] = 5;
        game.apply(Move::Match);
        assert!(game.drawing);
        assert_eq!(game.actor(), 1);
        let old = game.hands[1].clone();
        let choices = game.choices();
        assert_eq!(choices.len(), 32);
        assert!(choices[0].view.selection.as_ref().unwrap().cards.is_empty());
        let selection = &choices[5];
        assert_eq!(
            selection.view.selection.as_ref().unwrap().cards,
            vec![old[0].id, old[2].id]
        );
        game.apply(selection.op);
        assert_eq!(game.actor(), 0);
        assert_ne!(game.hands[1][0], old[0]);
        assert_ne!(game.hands[1][2], old[2]);
        for i in [1, 3, 4] {
            assert_eq!(game.hands[1][i], old[i]);
        }
        assert!(
            game.table(false).zones[0]
                .cards
                .iter()
                .all(|card| card.hidden)
        );
        let held = game.hands[0].clone();
        game.apply(Move::Draw(0));
        assert_eq!(game.hands[0], held);
        assert!(game.settled && game.reveal);
        assert_eq!(game.chips.iter().sum::<u32>(), 510);
    }

    #[test]
    fn stud_opens_on_the_low_upcard_then_the_best_exposed_cards() {
        let mut game = game(Variant::Stud);
        let lowest = usize::from(bring_in_key(game.hands[1][2]) < bring_in_key(game.hands[0][2]));
        assert_eq!(game.turn, lowest);
        assert_eq!(game.pot, 2);
        assert_eq!(game.choices()[0].view.label, "Bring in 5");
        assert_eq!(game.choices()[1].view.label, "Complete to 10");
        game.apply(Move::Match);
        assert_eq!(game.choices().last().unwrap().view.label, "Complete to 10");
        game.apply(Move::Match);
        assert_eq!(game.street, 1);
        assert_eq!(game.pot, 12);
        for street in 1..=4 {
            assert_eq!(game.street, street);
            let expected = usize::from(
                groups(stud_upcards(&game.hands[1])) >= groups(stud_upcards(&game.hands[0])),
            );
            assert_eq!(game.turn, expected);
            let table = game.table(false);
            let private = &table.zones[0];
            let public = &table.zones[1];
            assert_eq!(private.count, if street == 4 { 3 } else { 2 });
            assert!(private.cards.iter().all(|card| card.hidden));
            assert_eq!(public.count, (street + 1).min(4) as u32);
            assert!(public.cards.iter().all(|card| !card.hidden));
            game.apply(Move::Match);
            game.apply(Move::Match);
        }
        assert!(game.settled && game.reveal);
        assert_eq!(game.chips.iter().sum::<u32>(), 1000);
    }
}
