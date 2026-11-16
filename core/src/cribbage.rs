use crate::{
    Choice,
    card::{self, Card, Suit},
    wire::{
        self,
        zone::Layout::{Hand, Pile},
    },
};
use serde::Deserialize;

const NAMES: [&str; 2] = ["You", "Old James"];
const HAND: u32 = 5;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    target: u16,
}

impl Rules {
    pub fn validate(&self) -> Result<(), String> {
        if matches!(self.target, 61 | 121) {
            Ok(())
        } else {
            Err("Cribbage is played to 61 or 121 points.".into())
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Discard,
    Peg,
    Between,
    Finished,
}

#[derive(Clone, Copy)]
pub enum Move {
    Discard(u32),
    Play(u32),
    Go,
    Next,
}

pub struct Game {
    rules: Rules,
    seed: u64,
    deal: u64,
    dealer: usize,
    turn: usize,
    phase: Phase,
    scores: [u16; 2],
    hands: [Vec<Card>; 2],
    kept: [Vec<Card>; 2],
    crib: Vec<Card>,
    starter: Option<Card>,
    pending_starter: Card,
    count: u8,
    sequence: Vec<(usize, Card)>,
    go: [bool; 2],
    show_hands: bool,
    events: Vec<String>,
}

impl Game {
    pub fn new(rules: Rules, seed: u64) -> Self {
        let mut deck = card::deck(&Suit::ALL, 1, seed);
        let mut game = Self {
            rules,
            seed,
            deal: 0,
            dealer: (seed & 1) as usize,
            turn: 0,
            phase: Phase::Discard,
            scores: [0; 2],
            hands: Default::default(),
            kept: Default::default(),
            crib: Vec::new(),
            starter: None,
            pending_starter: deck.pop().unwrap(),
            count: 0,
            sequence: Vec::new(),
            go: [false; 2],
            show_hands: false,
            events: Vec::new(),
        };
        game.deal(&mut deck);
        game
    }

    fn deal(&mut self, deck: &mut Vec<Card>) {
        self.hands = Default::default();
        for _ in 0..6 {
            for seat in [1 - self.dealer, self.dealer] {
                self.hands[seat].push(deck.pop().expect("card for deal"));
            }
        }
        for hand in &mut self.hands {
            hand.sort_by_key(|card| (card.rank, card.suit as u8));
        }
        self.kept = Default::default();
        self.crib.clear();
        self.starter = None;
        self.turn = 0;
        self.phase = Phase::Discard;
        self.reset_count();
        self.show_hands = false;
        self.events = vec![format!("Dealer: {}", NAMES[self.dealer])];
    }

    pub fn actor(&self) -> usize {
        if matches!(self.phase, Phase::Discard | Phase::Peg) {
            self.turn
        } else {
            0
        }
    }

    pub fn choices(&self) -> Vec<Choice<Move>> {
        let available = &self.hands[self.turn];
        match self.phase {
            Phase::Discard => available
                .iter()
                .map(|card| {
                    Choice::new(
                        Move::Discard(card.id),
                        format!("Discard {} to the crib", card.name()),
                        HAND,
                        card.id,
                        0,
                    )
                })
                .collect(),
            Phase::Peg => {
                let cards: Vec<_> = available
                    .iter()
                    .filter(|card| self.count + value(**card) <= 31)
                    .map(|card| {
                        Choice::new(
                            Move::Play(card.id),
                            format!("Play {} · count {}", card.name(), self.count + value(*card)),
                            HAND,
                            card.id,
                            0,
                        )
                    })
                    .collect();
                if cards.is_empty() {
                    vec![Choice::new(Move::Go, "Go".into(), 0, 0, 0)]
                } else {
                    cards
                }
            }
            Phase::Between => vec![Choice::new(Move::Next, "Next deal".into(), 0, 0, 0)],
            Phase::Finished => Vec::new(),
        }
    }

    pub fn apply(&mut self, action: Move) {
        let seat = self.turn;
        match action {
            Move::Next => {
                self.dealer = 1 - self.dealer;
                self.deal = self.deal.wrapping_add(1);
                let mut deck = card::deck(&Suit::ALL, 1, self.seed.wrapping_add(self.deal));
                self.pending_starter = deck.pop().unwrap();
                self.deal(&mut deck);
            }
            Move::Discard(id) => {
                let card = self.remove(seat, id);
                self.crib.push(card);
                if self.hands[seat].len() == 4 {
                    self.events
                        .push(format!("{}: two cards to the crib", NAMES[seat]));
                    if seat == 0 {
                        self.turn = 1;
                    } else {
                        self.kept = self.hands.clone();
                        self.starter = Some(self.pending_starter);
                        self.events
                            .push(format!("Starter: {}", self.pending_starter.short()));
                        self.phase = Phase::Peg;
                        self.turn = 1 - self.dealer;
                        if self.pending_starter.rank == 11 {
                            self.award(self.dealer, 2, "jack starter");
                        }
                    }
                }
            }
            Move::Play(id) => self.play(seat, id),
            Move::Go => {
                self.go[seat] = true;
                self.events.push(format!("{}: Go", NAMES[seat]));
                let other = 1 - seat;
                if self.go[other] {
                    let last = self
                        .sequence
                        .last()
                        .expect("a card was played in this count")
                        .0;
                    if !self.award(last, 1, "go") {
                        self.end_count(last);
                    }
                } else {
                    self.turn = other;
                }
            }
        }
    }

    fn remove(&mut self, seat: usize, id: u32) -> Card {
        let index = self.hands[seat]
            .iter()
            .position(|card| card.id == id)
            .unwrap();
        self.hands[seat].remove(index)
    }

    fn play(&mut self, seat: usize, id: u32) {
        let card = self.remove(seat, id);
        self.count += value(card);
        self.sequence.push((seat, card));
        self.events.push(format!(
            "{}: {} · {}",
            NAMES[seat],
            card.short(),
            self.count
        ));
        let points = peg_points(&self.sequence, self.count);
        if self.award(seat, points, "pegging") {
            return;
        }
        if self.count == 31 {
            self.end_count(seat);
        } else if self.hands.iter().all(Vec::is_empty) {
            if !self.award(seat, 1, "last card") {
                self.end_count(seat);
            }
        } else if !self.go[1 - seat] {
            self.turn = 1 - seat;
        }
    }

    fn end_count(&mut self, last: usize) {
        self.reset_count();
        if self.hands.iter().all(Vec::is_empty) {
            self.score_deal();
        } else {
            self.turn = if self.hands[1 - last].is_empty() {
                last
            } else {
                1 - last
            };
        }
    }

    fn reset_count(&mut self) {
        self.count = 0;
        self.sequence.clear();
        self.go = [false; 2];
    }

    fn score_deal(&mut self) {
        self.show_hands = true;
        self.phase = Phase::Between;
        let starter = self.starter.expect("starter before count");
        for seat in [1 - self.dealer, self.dealer] {
            if self.award(seat, hand_points(&self.kept[seat], starter, false), "hand") {
                return;
            }
        }
        self.award(self.dealer, hand_points(&self.crib, starter, true), "crib");
    }

    fn award(&mut self, seat: usize, points: u16, reason: &str) -> bool {
        if points != 0 || matches!(reason, "hand" | "crib") {
            self.events
                .push(format!("{}: +{points} for {reason}", NAMES[seat]));
        }
        self.scores[seat] = (self.scores[seat] + points).min(self.rules.target);
        if self.scores[seat] == self.rules.target {
            self.phase = Phase::Finished;
            true
        } else {
            false
        }
    }

    pub fn table(&self, agent: bool) -> wire::Table {
        let viewer = usize::from(agent);
        let other = 1 - viewer;
        let shown = if self.show_hands {
            &self.kept
        } else {
            &self.hands
        };
        let hand = |seat: usize, id, row| {
            card::zone(
                id,
                row,
                format!(
                    "{} · {} points{}",
                    NAMES[seat],
                    self.scores[seat],
                    if self.dealer == seat {
                        " · Dealer"
                    } else {
                        ""
                    }
                ),
                Hand,
                shown[seat]
                    .iter()
                    .map(|card| {
                        if self.show_hands || seat == viewer {
                            card.view()
                        } else {
                            card::hidden()
                        }
                    })
                    .collect(),
                shown[seat].len(),
            )
        };
        let zones = vec![
            hand(other, 1, 0),
            card::zone(
                2,
                1,
                "Starter".into(),
                Pile,
                self.starter.map(|c| c.view()).into_iter().collect(),
                usize::from(self.starter.is_some()),
            ),
            card::zone(
                3,
                1,
                "Crib".into(),
                Hand,
                self.crib
                    .iter()
                    .map(|c| {
                        if self.show_hands {
                            c.view()
                        } else {
                            card::hidden()
                        }
                    })
                    .collect(),
                self.crib.len(),
            ),
            card::zone(
                4,
                1,
                format!("Count: {}", self.count),
                Hand,
                self.sequence.iter().map(|(_, c)| c.view()).collect(),
                self.sequence.len(),
            ),
            hand(viewer, HAND, 2),
        ];
        let status = match self.phase {
            Phase::Discard => format!(
                "{} · Discard {} to the crib",
                NAMES[self.turn],
                self.hands[self.turn].len() - 4
            ),
            Phase::Peg => format!("{} to play · Count {}", NAMES[self.turn], self.count),
            Phase::Between => "Deal scored".into(),
            Phase::Finished => {
                if self.scores[0] == self.rules.target {
                    "You win".into()
                } else {
                    "Old James wins".into()
                }
            }
        };
        let sequence = self
            .sequence
            .iter()
            .map(|(seat, card)| format!("{} {}", NAMES[*seat], card.short()))
            .collect::<Vec<_>>()
            .join(", ");
        let player_view = format!(
            "You are {}. Dealer: {}. Your playable cards: {}. Starter: {}. Count: {}. Cards in this count: {}. Score: You {}, Old James {}. {status}.",
            NAMES[viewer],
            NAMES[self.dealer],
            cards_text(&shown[viewer]),
            self.starter.map_or("face down".into(), Card::short),
            self.count,
            sequence,
            self.scores[0],
            self.scores[1],
        );
        wire::Table {
            zones,
            status: format!(
                "{status} · You {} · Old James {} · First to {}",
                self.scores[0], self.scores[1], self.rules.target
            ),
            agent_turn: self.actor() == 1,
            finished: self.phase == Phase::Finished,
            player_view,
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

fn value(card: Card) -> u8 {
    card.rank.min(10)
}

fn peg_points(sequence: &[(usize, Card)], count: u8) -> u16 {
    let rank = sequence.last().unwrap().1.rank;
    let repeated = sequence
        .iter()
        .rev()
        .take_while(|(_, card)| card.rank == rank)
        .count() as u16;
    let run = (3..=sequence.len())
        .rev()
        .find(|&len| {
            let ranks = sequence[sequence.len() - len..]
                .iter()
                .map(|(_, card)| card.rank);
            is_run(ranks)
        })
        .unwrap_or(0);
    u16::from(matches!(count, 15 | 31)) * 2 + repeated * (repeated - 1) + run as u16
}

fn is_run(ranks: impl Iterator<Item = u8>) -> bool {
    let mut mask = 0u16;
    let mut count = 0;
    for rank in ranks {
        let bit = 1 << (rank - 1);
        if mask & bit != 0 {
            return false;
        }
        mask |= bit;
        count += 1;
    }
    mask >> mask.trailing_zeros() == (1 << count) - 1
}

fn hand_points(hand: &[Card], starter: Card, crib: bool) -> u16 {
    let cards = [hand[0], hand[1], hand[2], hand[3], starter];
    let mut points = 0;
    let mut longest_run = 0;
    let mut run_points = 0;
    for subset in 1u8..32 {
        let chosen = || {
            cards
                .iter()
                .copied()
                .enumerate()
                .filter(|(i, _)| subset & (1 << i) != 0)
                .map(|(_, card)| card)
        };
        if chosen().map(value).sum::<u8>() == 15 {
            points += 2;
        }
        let len = subset.count_ones() as u16;
        if len == 2 {
            let mut pair = chosen();
            if pair.next().unwrap().rank == pair.next().unwrap().rank {
                points += 2;
            }
        } else if len >= 3 && len >= longest_run && is_run(chosen().map(|card| card.rank)) {
            if len > longest_run {
                longest_run = len;
                run_points = 0;
            }
            run_points += len;
        }
    }
    if hand.iter().all(|card| card.suit == hand[0].suit) {
        if starter.suit == hand[0].suit {
            points += 5;
        } else if !crib {
            points += 4;
        }
    }
    points
        + run_points
        + u16::from(
            hand.iter()
                .any(|card| card.rank == 11 && card.suit == starter.suit),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(rank: u8, suit: Suit) -> Card {
        Card {
            id: suit as u32 * 13 + u32::from(rank),
            rank,
            suit,
        }
    }

    #[test]
    fn counts_the_29_hand_and_multiplicity_of_runs() {
        use Suit::*;
        let hand = [c(5, Clubs), c(5, Diamonds), c(5, Hearts), c(11, Spades)];
        assert_eq!(hand_points(&hand, c(5, Spades), false), 29);
        let hand = [c(2, Clubs), c(2, Diamonds), c(3, Hearts), c(4, Spades)];
        assert_eq!(hand_points(&hand, c(4, Clubs), false), 18);
        let hand = [c(1, Clubs), c(2, Diamonds), c(3, Hearts), c(4, Spades)];
        assert_eq!(hand_points(&hand, c(5, Clubs), false), 7);
    }

    #[test]
    fn a_four_card_flush_only_counts_in_the_hand() {
        let hand = [1, 3, 6, 9].map(|rank| c(rank, Suit::Clubs));
        let other_suit = c(12, Suit::Hearts);
        assert_eq!(
            hand_points(&hand, other_suit, false),
            hand_points(&hand, other_suit, true) + 4
        );
        let same_suit = c(12, Suit::Clubs);
        assert_eq!(
            hand_points(&hand, same_suit, true),
            hand_points(&hand, other_suit, true) + 5
        );
    }

    #[test]
    fn pegging_counts_runs_in_play_order_and_combines_points() {
        let seq = [
            (0, c(7, Suit::Clubs)),
            (1, c(9, Suit::Hearts)),
            (0, c(8, Suit::Clubs)),
        ];
        assert_eq!(peg_points(&seq, 24), 3);
        let seq = [
            (0, c(5, Suit::Clubs)),
            (1, c(5, Suit::Hearts)),
            (0, c(5, Suit::Spades)),
        ];
        assert_eq!(peg_points(&seq, 15), 8);
        let seq = [10, 9, 6, 6]
            .into_iter()
            .zip(Suit::ALL)
            .map(|(rank, suit)| (0, c(rank, suit)))
            .collect::<Vec<_>>();
        assert_eq!(peg_points(&seq, 31), 4);
    }

    #[test]
    fn go_pays_the_last_player_and_passes_the_lead() {
        let mut game = Game::new(Rules { target: 121 }, 0);
        game.phase = Phase::Peg;
        game.turn = 0;
        game.count = 23;
        game.sequence = vec![
            (1, c(12, Suit::Clubs)),
            (0, c(13, Suit::Clubs)),
            (1, c(3, Suit::Clubs)),
        ];
        let five = c(5, Suit::Hearts);
        game.hands = [vec![five, c(8, Suit::Hearts)], vec![c(10, Suit::Spades)]];
        game.apply(Move::Play(five.id));
        assert_eq!(game.turn, 1);
        assert!(matches!(game.choices()[0].op, Move::Go));
        game.apply(Move::Go);
        assert_eq!(game.turn, 0);
        assert!(matches!(game.choices()[0].op, Move::Go));
        game.apply(Move::Go);
        assert_eq!(game.scores, [1, 0]);
        assert_eq!(game.turn, 1);
        assert_eq!(game.count, 0);
        assert!(game.sequence.is_empty());
    }

    #[test]
    fn nondealer_counts_first_and_wins_before_dealer_or_crib_score() {
        let mut game = Game::new(Rules { target: 121 }, 0);
        game.scores = [120, 120];
        game.kept[1] = [1, 2, 3, 4].map(|rank| c(rank, Suit::Clubs)).to_vec();
        game.starter = Some(c(5, Suit::Hearts));
        game.score_deal();
        assert_eq!(game.scores, [120, 121]);
        assert!(game.phase == Phase::Finished);
        assert!(game.choices().is_empty());
    }
}
