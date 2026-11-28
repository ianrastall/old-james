use crate::{
    Choice,
    card::{self, Card, Suit},
    wire,
};
use serde::Deserialize;

mod advice;

const STOCK: u32 = 1;
const DISCARD: u32 = 2;
const HAND: u32 = 3;
const MELD: u32 = 10;
const NAMES: [&str; 2] = ["You", "Old James"];

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Basic,
    Gin,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    mode: Mode,
    target: u32,
    #[serde(default)]
    knock_limit: u32,
    #[serde(default)]
    gin_bonus: u32,
    #[serde(default)]
    undercut: u32,
}

impl Rules {
    pub fn validate(&self) -> Result<(), String> {
        let bonuses = [self.knock_limit, self.gin_bonus, self.undercut];
        if !(1..=1000).contains(&self.target)
            || match self.mode {
                Mode::Basic => bonuses != [0; 3],
                Mode::Gin => !bonuses.iter().all(|n| (1..=100).contains(n)),
            }
        {
            Err("Invalid Rummy score or bonus settings.".into())
        } else {
            Ok(())
        }
    }

    /// Both modes have a built-in opponent.
    pub fn advised(&self) -> bool {
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Move {
    Pass,
    Draw,
    Take,
    Discard(u32),
    Knock(u32),
    Meld(u16),
    Layoff(u32, usize),
    Next,
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Offer,
    Draw,
    Play,
    Between,
    Finished,
}

pub struct Game {
    rules: Rules,
    seed: u64,
    deal: u32,
    dealer: usize,
    turn: usize,
    stock: Vec<Card>,
    discard: Vec<Card>,
    hands: [Vec<Card>; 2],
    groups: Vec<(usize, Vec<Card>)>,
    showed: [bool; 2],
    had_meld: bool,
    taken: Option<u32>,
    // Cards each seat took from the discard pile and discarded this deal.
    picked: [Vec<Card>; 2],
    tossed: [Vec<Card>; 2],
    must_draw: bool,
    phase: Phase,
    scores: [u32; 2],
    events: Vec<String>,
    result: String,
    reveal: bool,
}

impl Game {
    pub fn new(rules: Rules, seed: u64) -> Self {
        let mut game = Self {
            rules,
            seed,
            deal: 0,
            dealer: 1,
            turn: 0,
            stock: vec![],
            discard: vec![],
            hands: Default::default(),
            groups: vec![],
            showed: [false; 2],
            had_meld: false,
            taken: None,
            picked: Default::default(),
            tossed: Default::default(),
            must_draw: false,
            phase: Phase::Between,
            scores: [0; 2],
            events: vec![],
            result: String::new(),
            reveal: false,
        };
        game.deal();
        game
    }

    fn deal(&mut self) {
        self.deal += 1;
        self.turn = 1 - self.dealer;
        self.stock = card::deck(&Suit::ALL, 1, self.seed.wrapping_add(u64::from(self.deal)));
        self.hands = Default::default();
        for _ in 0..10 {
            for seat in [self.turn, self.dealer] {
                self.hands[seat].push(self.stock.pop().unwrap());
            }
        }
        for hand in &mut self.hands {
            sort(hand);
        }
        self.discard = vec![self.stock.pop().unwrap()];
        self.groups.clear();
        self.showed = [false; 2];
        self.had_meld = false;
        self.taken = None;
        self.picked = Default::default();
        self.tossed = Default::default();
        self.must_draw = false;
        self.reveal = false;
        self.events.clear();
        self.phase = if self.rules.mode == Mode::Gin {
            Phase::Offer
        } else {
            Phase::Draw
        };
    }

    pub fn actor(&self) -> usize {
        if matches!(self.phase, Phase::Between | Phase::Finished) {
            0
        } else {
            self.turn
        }
    }

    pub fn choices(&self) -> Vec<Choice<Move>> {
        let simple = |m, label: &str, source| Choice::new(m, label.into(), source, 0, 0);
        match self.phase {
            Phase::Offer => vec![
                simple(Move::Take, "Take the upcard", DISCARD),
                simple(Move::Pass, "Pass on the upcard", 0),
            ],
            Phase::Draw => {
                let mut moves = vec![simple(
                    Move::Draw,
                    if self.stock.is_empty() {
                        "Turn over discard pile and draw"
                    } else {
                        "Draw from stock"
                    },
                    STOCK,
                )];
                if !self.must_draw
                    && let Some(c) = self.discard.last()
                {
                    moves.push(simple(Move::Take, &format!("Take {}", c.name()), DISCARD));
                }
                moves
            }
            Phase::Play => {
                let hand = &self.hands[self.turn];
                let mut moves = Vec::new();
                for &card in hand {
                    if Some(card.id) == self.taken {
                        continue;
                    }
                    moves.push(Choice::new(
                        Move::Discard(card.id),
                        format!("Discard {}", card.name()),
                        HAND,
                        card.id,
                        DISCARD,
                    ));
                    if self.rules.mode == Mode::Gin {
                        let keep: Vec<_> =
                            hand.iter().copied().filter(|c| c.id != card.id).collect();
                        let result = best(&keep, &[]).0;
                        if result <= self.rules.knock_limit {
                            moves.push(Choice::new(
                                Move::Knock(card.id),
                                format!(
                                    "{} and discard {}",
                                    if result == 0 {
                                        "Go gin".into()
                                    } else {
                                        format!("Knock ({result})")
                                    },
                                    card.name()
                                ),
                                HAND,
                                card.id,
                                0,
                            ));
                        }
                    }
                }
                if self.rules.mode == Mode::Basic {
                    for mask in melds(hand) {
                        let group = select(hand, mask);
                        if let Some(last) = self.last_taken_after(&group) {
                            if !fits(last, &group)
                                && !self.groups.iter().any(|(_, old)| fits(last, old))
                            {
                                continue;
                            }
                        }
                        moves.push(simple(
                            Move::Meld(mask),
                            &format!("Meld {}", cards_text(&group)),
                            0,
                        ));
                    }
                    for (i, (_, group)) in self.groups.iter().enumerate() {
                        for &card in hand {
                            if fits(card, group) {
                                if let Some(last) = self.last_taken_after(&[card]) {
                                    let mut extended = group.clone();
                                    extended.push(card);
                                    sort(&mut extended);
                                    if !fits(last, &extended)
                                        && !self
                                            .groups
                                            .iter()
                                            .enumerate()
                                            .any(|(j, (_, old))| i != j && fits(last, old))
                                    {
                                        continue;
                                    }
                                }
                                moves.push(Choice::new(
                                    Move::Layoff(card.id, i),
                                    format!("Lay off {} on {}", card.short(), cards_text(group)),
                                    HAND,
                                    card.id,
                                    MELD + i as u32,
                                ));
                            }
                        }
                    }
                }
                moves
            }
            Phase::Between => vec![simple(Move::Next, "Next deal", 0)],
            Phase::Finished => vec![],
        }
    }

    fn last_taken_after(&self, leaving: &[Card]) -> Option<Card> {
        let hand = &self.hands[self.turn];
        if hand.len() != leaving.len() + 1 {
            return None;
        }
        hand.iter()
            .copied()
            .find(|card| Some(card.id) == self.taken && !leaving.contains(card))
    }

    pub fn apply(&mut self, action: Move) {
        let seat = self.turn;
        let name = NAMES[seat];
        match action {
            Move::Next => self.deal(),
            Move::Pass => {
                self.events.push(format!("{name} passes on the upcard"));
                if seat == self.dealer {
                    self.phase = Phase::Draw;
                    self.must_draw = true;
                }
                self.turn = 1 - seat;
            }
            Move::Draw | Move::Take => {
                let take = matches!(action, Move::Take);
                let card = if take {
                    let card = self.discard.pop().unwrap();
                    self.events.push(format!("{name} takes {}", card.short()));
                    self.picked[seat].push(card);
                    card
                } else {
                    if self.stock.is_empty() {
                        self.stock.extend(self.discard.drain(..).rev());
                    }
                    self.events.push(format!("{name} draws from stock"));
                    self.stock.pop().unwrap()
                };
                self.taken = take.then_some(card.id);
                self.had_meld = self.showed[seat];
                self.hands[seat].push(card);
                sort(&mut self.hands[seat]);
                self.phase = Phase::Play;
            }
            Move::Discard(id) | Move::Knock(id) => {
                let card = self.remove(id);
                self.discard.push(card);
                self.tossed[seat].push(card);
                self.events
                    .push(format!("{name} discards {}", card.short()));
                if matches!(action, Move::Knock(_)) {
                    self.knock();
                } else if self.hands[seat].is_empty() {
                    self.basic_out();
                } else if self.rules.mode == Mode::Gin && self.stock.len() <= 2 {
                    self.result = "Stock reduced to two · no score".into();
                    self.dealer = 1 - self.dealer;
                    self.phase = Phase::Between;
                } else {
                    self.phase = Phase::Draw;
                    self.must_draw = false;
                    self.turn = 1 - seat;
                }
            }
            Move::Meld(mask) => {
                let group = select(&self.hands[seat], mask);
                self.events
                    .push(format!("{name} melds {}", cards_text(&group)));
                for card in &group {
                    self.remove(card.id);
                }
                self.groups.push((seat, group));
                self.showed[seat] = true;
                if self.hands[seat].is_empty() {
                    self.basic_out();
                }
            }
            Move::Layoff(id, group) => {
                let card = self.remove(id);
                self.groups[group].1.push(card);
                sort(&mut self.groups[group].1);
                self.events
                    .push(format!("{name} lays off {}", card.short()));
                self.showed[seat] = true;
                if self.hands[seat].is_empty() {
                    self.basic_out();
                }
            }
        }
        if self.events.len() > 160 {
            self.events.drain(..self.events.len() - 160);
        }
    }

    fn remove(&mut self, id: u32) -> Card {
        let hand = &mut self.hands[self.turn];
        let index = hand.iter().position(|c| c.id == id).unwrap();
        hand.remove(index)
    }

    fn basic_out(&mut self) {
        let count: u32 = self.hands[1 - self.turn].iter().copied().map(value).sum();
        self.award(
            self.turn,
            if self.had_meld { count } else { count * 2 },
            if self.had_meld { "Goes out" } else { "Rummy" },
        );
    }

    fn knock(&mut self) {
        let knocker = self.turn;
        let other = 1 - knocker;
        let (knock, masks) = best(&self.hands[knocker], &[]);
        let shown = masks
            .into_iter()
            .map(|m| select(&self.hands[knocker], m))
            .collect::<Vec<_>>();
        let (defend, _) = best(&self.hands[other], if knock == 0 { &[] } else { &shown });
        self.events.push(format!(
            "Deadwood: {} {knock} · {} {defend}",
            NAMES[knocker], NAMES[other]
        ));
        self.groups
            .extend(shown.into_iter().map(|group| (knocker, group)));
        if knock == 0 {
            self.award(knocker, self.rules.gin_bonus + defend, "Gin");
        } else if knock < defend {
            self.award(knocker, defend - knock, "Knock");
        } else {
            self.award(other, self.rules.undercut + knock - defend, "Undercut");
        }
    }

    fn award(&mut self, winner: usize, points: u32, action: &str) {
        self.scores[winner] += points;
        self.dealer = winner;
        self.reveal = true;
        let won = self.scores[winner] >= self.rules.target;
        self.phase = if won { Phase::Finished } else { Phase::Between };
        self.result = format!(
            "{action} · {} scores {points}{}",
            NAMES[winner],
            if won { " · Match over" } else { "" }
        );
    }

    pub fn table(&self, agent: bool) -> wire::Table {
        use wire::zone::Layout::{Hand, Pile};
        let viewer = usize::from(agent);
        let other = 1 - viewer;
        let discard = self.discard.last();
        let mut zones = vec![
            card::zone(
                4,
                0,
                NAMES[other].into(),
                Hand,
                self.hands[other]
                    .iter()
                    .map(|c| {
                        if self.reveal {
                            c.view()
                        } else {
                            card::hidden()
                        }
                    })
                    .collect(),
                self.hands[other].len(),
            ),
            card::zone(
                STOCK,
                1,
                "Stock".into(),
                Pile,
                if self.stock.is_empty() {
                    vec![]
                } else {
                    vec![card::hidden()]
                },
                self.stock.len(),
            ),
            card::zone(
                DISCARD,
                1,
                "Discard".into(),
                Pile,
                discard.map(|c| c.view()).into_iter().collect(),
                self.discard.len(),
            ),
            card::zone(
                HAND,
                2,
                NAMES[viewer].into(),
                Hand,
                self.hands[viewer].iter().map(|c| c.view()).collect(),
                self.hands[viewer].len(),
            ),
        ];
        for (i, (seat, group)) in self.groups.iter().enumerate() {
            zones.push(card::zone(
                MELD + i as u32,
                3 + (i / 4) as u32,
                format!("{} · Meld {}", NAMES[*seat], i + 1),
                Hand,
                group.iter().map(|c| c.view()).collect(),
                group.len(),
            ));
        }
        let current = NAMES[self.turn];
        let status = match self.phase {
            Phase::Offer => format!("{current} may take the upcard or pass"),
            Phase::Draw => format!("{current} to draw"),
            Phase::Play => format!("{current} to play"),
            _ => self.result.clone(),
        };
        let score = format!("You {} · Old James {}", self.scores[0], self.scores[1]);
        let groups = self
            .groups
            .iter()
            .map(|(seat, cards)| format!("{}: {}", NAMES[*seat], cards_text(cards)))
            .collect::<Vec<_>>()
            .join("; ");
        let player_view = format!(
            "You are {}. Your hand: {}. Stock: {} cards. Discard: {}. Melds: {groups}. {score}. {status}.",
            NAMES[viewer],
            cards_text(&self.hands[viewer]),
            self.stock.len(),
            discard.map_or("empty".into(), |c| c.short())
        );
        wire::Table {
            zones,
            status: format!("{status}   |   {score}"),
            player_view,
            agent_turn: self.actor() == 1,
            finished: self.phase == Phase::Finished,
            events: self.events.clone(),
            ..Default::default()
        }
    }
}

fn sort(cards: &mut [Card]) {
    cards.sort_by_key(|c| (c.suit as u8, c.rank));
}
fn cards_text(cards: &[Card]) -> String {
    cards
        .iter()
        .map(|c| c.short())
        .collect::<Vec<_>>()
        .join(" ")
}
fn value(card: Card) -> u32 {
    u32::from(card.rank.min(10))
}

fn select(cards: &[Card], mask: u16) -> Vec<Card> {
    cards
        .iter()
        .enumerate()
        .filter(|(i, _)| mask & (1 << i) != 0)
        .map(|(_, &c)| c)
        .collect()
}

fn melds(cards: &[Card]) -> Vec<u16> {
    let mut matches = Vec::new();
    for rank in 1..=13 {
        let mask = cards
            .iter()
            .enumerate()
            .filter(|(_, c)| c.rank == rank)
            .fold(0u16, |bits, (i, _)| bits | 1 << i);
        if mask.count_ones() >= 3 {
            matches.push(mask);
        }
        if mask.count_ones() == 4 {
            matches.extend(
                (0..cards.len())
                    .filter(|&i| mask & (1 << i) > 0)
                    .map(|i| mask ^ (1 << i)),
            );
        }
    }
    for (i, first) in cards.iter().enumerate() {
        let mut mask = 1 << i;
        let mut next = first.rank + 1;
        for (j, card) in cards.iter().enumerate().skip(i + 1) {
            if card.suit != first.suit || card.rank != next {
                break;
            }
            mask |= 1 << j;
            next += 1;
            if next - first.rank >= 3 {
                matches.push(mask);
            }
        }
    }
    matches
}

fn fits(card: Card, meld: &[Card]) -> bool {
    if meld[0].rank == meld[1].rank {
        meld.len() == 3 && card.rank == meld[0].rank
    } else {
        card.suit == meld[0].suit
            && (card.rank + 1 == meld[0].rank || card.rank == meld.last().unwrap().rank + 1)
    }
}

fn best(cards: &[Card], layoffs: &[Vec<Card>]) -> (u32, Vec<u16>) {
    fn reduce(cards: &[Card], remaining: u16, groups: &mut [Vec<Card>]) -> u32 {
        let mut score: u32 = cards
            .iter()
            .enumerate()
            .filter(|(i, _)| remaining & (1 << i) > 0)
            .map(|(_, &c)| value(c))
            .sum();
        for (i, &card) in cards.iter().enumerate() {
            if remaining & (1 << i) == 0 {
                continue;
            }
            for j in 0..groups.len() {
                if !fits(card, &groups[j]) {
                    continue;
                }
                groups[j].push(card);
                sort(&mut groups[j]);
                score = score.min(reduce(cards, remaining ^ (1 << i), groups));
                groups[j].retain(|c| c.id != card.id);
            }
        }
        score
    }
    fn search(
        cards: &[Card],
        options: &[u16],
        remaining: u16,
        selected: &mut Vec<u16>,
        groups: &mut [Vec<Card>],
        best: &mut (u32, Vec<u16>),
    ) {
        let score = reduce(cards, remaining, groups);
        if score < best.0 {
            *best = (score, selected.clone());
        }
        if best.0 == 0 {
            return;
        }
        for (i, &mask) in options
            .iter()
            .enumerate()
            .filter(|(_, m)| remaining & **m == **m)
        {
            selected.push(mask);
            search(
                cards,
                &options[i + 1..],
                remaining ^ mask,
                selected,
                groups,
                best,
            );
            selected.pop();
        }
    }
    let mut optimum = (u32::MAX, Vec::new());
    search(
        cards,
        &melds(cards),
        (1 << cards.len()) - 1,
        &mut Vec::new(),
        &mut layoffs.to_vec(),
        &mut optimum,
    );
    optimum
}
