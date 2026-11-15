use crate::{
    Choice,
    card::{self, Card, Suit},
    wire,
};
use serde::Deserialize;

const HAND: u32 = 10;
const TRICK: u32 = 20;
const SEATS: [&str; 4] = ["South", "West", "North", "East"];

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Hearts,
    Spades,
    Bridge,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    mode: Mode,
    target: Option<i32>,
}

impl Rules {
    pub fn validate(&self) -> Result<(), String> {
        match (self.mode, self.target) {
            (Mode::Bridge, None) => Ok(()),
            (Mode::Hearts | Mode::Spades, Some(1..=1000)) => Ok(()),
            _ => Err("Invalid trick-taking score target.".into()),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Pass,
    Bid,
    Play,
    Between,
    Finished,
}

#[derive(Clone, Copy)]
pub enum Move {
    Card(u32),
    Bid(u8),
    Pass,
    Double,
    Redouble,
    Next,
}

#[derive(Clone, Copy)]
struct Contract {
    value: u8,
    owner: usize,
    multiplier: u8,
}

impl Contract {
    fn level(self) -> u8 {
        self.value / 5 + 1
    }
    fn strain(self) -> usize {
        (self.value % 5) as usize
    }
    fn label(self) -> String {
        let double = match self.multiplier {
            2 => " doubled",
            4 => " redoubled",
            _ => "",
        };
        format!("{}{}", bid_label(self.value), double)
    }
}

fn bid_label(value: u8) -> String {
    format!(
        "{}{}",
        value / 5 + 1,
        ["♣", "♦", "♥", "♠", "NT"][(value % 5) as usize]
    )
}

pub struct Game {
    rules: Rules,
    seed: u64,
    deal: u32,
    phase: Phase,
    turn: usize,
    hands: [Vec<Card>; 4],
    passed: [Vec<Card>; 4],
    trick: Vec<(usize, Card)>,
    last_trick: Vec<(usize, Card)>,
    taken: [Vec<Card>; 4],
    tricks: [i32; 4],
    bids: [i32; 4],
    scores: [i32; 4],
    bags: [i32; 2],
    broken: bool,
    events: Vec<String>,
    contract: Option<Contract>,
    first_bidders: [[Option<usize>; 5]; 2],
    consecutive_passes: usize,
    declarer: Option<usize>,
    status: String,
}

impl Game {
    pub fn new(rules: Rules, seed: u64) -> Self {
        let mut game = Self {
            rules,
            seed,
            deal: 0,
            phase: Phase::Between,
            turn: 0,
            hands: Default::default(),
            passed: Default::default(),
            trick: Vec::new(),
            last_trick: Vec::new(),
            taken: Default::default(),
            tricks: [0; 4],
            bids: [-1; 4],
            scores: [0; 4],
            bags: [0; 2],
            broken: false,
            events: Vec::new(),
            contract: None,
            first_bidders: [[None; 5]; 2],
            consecutive_passes: 0,
            declarer: None,
            status: String::new(),
        };
        game.deal();
        game
    }

    fn deal(&mut self) {
        self.turn = if self.rules.mode == Mode::Bridge {
            (self.deal as usize + 2) % 4
        } else {
            (self.deal % 4) as usize
        };
        self.deal += 1;
        self.phase = match self.rules.mode {
            Mode::Hearts if self.deal % 4 == 0 => Phase::Play,
            Mode::Hearts => Phase::Pass,
            _ => Phase::Bid,
        };
        self.broken = false;
        self.trick.clear();
        self.last_trick.clear();
        self.taken = Default::default();
        self.passed = Default::default();
        self.tricks = [0; 4];
        self.bids = [-1; 4];
        self.contract = None;
        self.declarer = None;
        self.first_bidders = [[None; 5]; 2];
        self.consecutive_passes = 0;
        self.events.clear();
        let deck = card::deck(&Suit::ALL, 1, self.seed.wrapping_add(u64::from(self.deal)));
        self.hands = std::array::from_fn(|seat| {
            let mut hand = deck
                .iter()
                .skip(seat)
                .step_by(4)
                .copied()
                .collect::<Vec<_>>();
            Self::sort(&mut hand);
            hand
        });
        if self.rules.mode == Mode::Hearts && self.phase == Phase::Play {
            self.find_opening_club();
        }
    }

    fn sort(hand: &mut [Card]) {
        hand.sort_by_key(|card| (card.suit as usize, card.high()));
    }

    fn find_opening_club(&mut self) {
        self.turn = self
            .hands
            .iter()
            .position(|hand| hand.iter().any(|c| c.suit == Suit::Clubs && c.rank == 2))
            .unwrap();
    }

    pub fn actor(&self) -> usize {
        if matches!(self.phase, Phase::Between | Phase::Finished) {
            return 0;
        }
        if self.phase == Phase::Play && self.declarer.is_some_and(|d| (d + 2) % 4 == self.turn) {
            return self.declarer.unwrap();
        }
        self.turn
    }

    fn pass_offset(&self) -> usize {
        [0, 1, 3, 2][self.deal as usize % 4]
    }

    pub fn choices(&self) -> Vec<Choice<Move>> {
        let hand = &self.hands[self.turn];
        match self.phase {
            Phase::Pass => hand
                .iter()
                .map(|c| {
                    Choice::new(
                        Move::Card(c.id),
                        format!("Pass {}", c.name()),
                        HAND + self.turn as u32,
                        c.id,
                        0,
                    )
                })
                .collect(),
            Phase::Bid if self.rules.mode == Mode::Bridge => {
                let make = |m, label: &str| Choice::new(m, label.into(), 0, 0, 0);
                let mut actions = vec![make(Move::Pass, "Pass")];
                let first = self.contract.map_or(0, |b| b.value + 1);
                actions.extend((first..35).map(|n| make(Move::Bid(n), &bid_label(n))));
                if let Some(bid) = self.contract {
                    if bid.multiplier == 1 && bid.owner % 2 != self.turn % 2 {
                        actions.push(make(Move::Double, "Double"));
                    }
                    if bid.multiplier == 2 && bid.owner % 2 == self.turn % 2 {
                        actions.push(make(Move::Redouble, "Redouble"));
                    }
                }
                actions
            }
            Phase::Bid => (0..=13)
                .map(|n| {
                    Choice::new(
                        Move::Bid(n),
                        if n == 0 {
                            "Bid nil".into()
                        } else {
                            format!("Bid {n}")
                        },
                        0,
                        0,
                        0,
                    )
                })
                .collect(),
            Phase::Between => vec![Choice::new(Move::Next, "Next deal".into(), 0, 0, 0)],
            Phase::Finished => vec![],
            Phase::Play => self
                .legal_cards(hand)
                .into_iter()
                .map(|c| {
                    Choice::new(
                        Move::Card(c.id),
                        format!("Play {}", c.name()),
                        HAND + self.turn as u32,
                        c.id,
                        TRICK,
                    )
                })
                .collect(),
        }
    }

    fn legal_cards(&self, hand: &[Card]) -> Vec<Card> {
        let first = self.tricks.iter().all(|&n| n == 0);
        if let Some((_, lead)) = self.trick.first() {
            let suited: Vec<_> = hand
                .iter()
                .copied()
                .filter(|c| c.suit == lead.suit)
                .collect();
            if !suited.is_empty() {
                return suited;
            }
            if first && self.rules.mode == Mode::Hearts {
                let clean: Vec<_> = hand
                    .iter()
                    .copied()
                    .filter(|c| heart_points(*c) == 0)
                    .collect();
                if !clean.is_empty() {
                    return clean;
                }
            }
            return hand.to_vec();
        }
        match self.rules.mode {
            Mode::Hearts if first => hand
                .iter()
                .copied()
                .filter(|c| c.suit == Suit::Clubs && c.rank == 2)
                .collect(),
            Mode::Hearts if !self.broken && hand.iter().any(|c| c.suit != Suit::Hearts) => hand
                .iter()
                .copied()
                .filter(|c| c.suit != Suit::Hearts)
                .collect(),
            Mode::Spades if !self.broken && hand.iter().any(|c| c.suit != Suit::Spades) => hand
                .iter()
                .copied()
                .filter(|c| c.suit != Suit::Spades)
                .collect(),
            _ => hand.to_vec(),
        }
    }

    pub fn apply(&mut self, action: Move) {
        match action {
            Move::Next => self.deal(),
            Move::Card(id) => {
                let index = self.hands[self.turn]
                    .iter()
                    .position(|c| c.id == id)
                    .unwrap();
                let card = self.hands[self.turn].remove(index);
                if self.phase == Phase::Pass {
                    self.pass_card(card);
                } else {
                    self.play_card(card);
                }
            }
            Move::Bid(bid) if self.rules.mode == Mode::Spades => {
                self.bids[self.turn] = i32::from(bid);
                self.events.push(format!(
                    "{} bids {}",
                    SEATS[self.turn],
                    if bid == 0 {
                        "nil".into()
                    } else {
                        bid.to_string()
                    }
                ));
                self.turn = (self.turn + 1) % 4;
                if self.bids.iter().all(|n| *n >= 0) {
                    self.phase = Phase::Play;
                }
            }
            Move::Bid(_) | Move::Pass | Move::Double | Move::Redouble => self.auction(action),
        }
    }

    fn pass_card(&mut self, card: Card) {
        self.passed[self.turn].push(card);
        if self.passed[self.turn].len() < 3 {
            return;
        }
        self.turn = (self.turn + 1) % 4;
        if self.passed.iter().all(|cards| cards.len() == 3) {
            let offset = self.pass_offset();
            for (i, cards) in self.passed.iter_mut().enumerate() {
                let receiver = (i + offset) % 4;
                self.hands[receiver].append(cards);
                Self::sort(&mut self.hands[receiver]);
            }
            self.phase = Phase::Play;
            self.find_opening_club();
            self.events.push("The pass is complete.".into());
        }
    }

    fn auction(&mut self, action: Move) {
        let label = match action {
            Move::Pass => {
                self.consecutive_passes += 1;
                "passes".into()
            }
            Move::Bid(value) => {
                let b = Contract {
                    value,
                    owner: self.turn,
                    multiplier: 1,
                };
                self.first_bidders[self.turn % 2][b.strain()].get_or_insert(self.turn);
                self.contract = Some(b);
                self.consecutive_passes = 0;
                format!("bids {}", b.label())
            }
            Move::Double | Move::Redouble => {
                let bid = self.contract.as_mut().unwrap();
                bid.multiplier *= 2;
                self.consecutive_passes = 0;
                if matches!(action, Move::Double) {
                    "doubles".into()
                } else {
                    "redoubles".into()
                }
            }
            _ => unreachable!(),
        };
        self.events.push(format!("{} {label}", SEATS[self.turn]));
        self.turn = (self.turn + 1) % 4;
        if let Some(bid) = self.contract {
            if self.consecutive_passes == 3 {
                let declarer = self.first_bidders[bid.owner % 2][bid.strain()].unwrap();
                self.declarer = Some(declarer);
                self.phase = Phase::Play;
                self.turn = (declarer + 1) % 4;
                self.events
                    .push(format!("{} declares {}", SEATS[declarer], bid.label()));
            }
        } else if self.consecutive_passes == 4 {
            self.status = "Passed out · no score".into();
            self.phase = Phase::Between;
        }
    }

    fn play_card(&mut self, card: Card) {
        self.broken |= match self.rules.mode {
            Mode::Hearts => heart_points(card) > 0,
            Mode::Spades => card.suit == Suit::Spades,
            Mode::Bridge => false,
        };
        self.events
            .push(format!("{} plays {}", SEATS[self.turn], card.short()));
        self.trick.push((self.turn, card));
        self.turn = (self.turn + 1) % 4;
        if self.trick.len() != 4 {
            return;
        }
        let trump = match self.rules.mode {
            Mode::Spades => Some(Suit::Spades),
            Mode::Bridge => self
                .contract
                .and_then(|b| Suit::ALL.get(b.strain()).copied()),
            Mode::Hearts => None,
        };
        let lead = self.trick[0].1.suit;
        let winner = self
            .trick
            .iter()
            .max_by_key(|(_, card)| {
                (
                    u8::from(Some(card.suit) == trump) * 2 + u8::from(card.suit == lead),
                    card.high(),
                )
            })
            .unwrap()
            .0;
        self.events
            .push(format!("{} takes the trick", SEATS[winner]));
        self.tricks[winner] += 1;
        self.taken[winner].extend(self.trick.iter().map(|&(_, c)| c));
        self.last_trick = std::mem::take(&mut self.trick);
        self.turn = winner;
        if self.hands[0].is_empty() {
            self.score_deal();
        }
    }

    fn score_deal(&mut self) {
        self.phase = Phase::Between;
        match self.rules.mode {
            Mode::Hearts => {
                let points: [i32; 4] =
                    std::array::from_fn(|i| self.taken[i].iter().map(|c| heart_points(*c)).sum());
                let moon = points.iter().position(|&p| p == 26);
                for (i, point) in points.into_iter().enumerate() {
                    self.scores[i] += moon.map_or(point, |seat| if i == seat { 0 } else { 26 });
                }
                if self.scores.iter().any(|v| *v >= self.rules.target.unwrap()) {
                    let low = *self.scores.iter().min().unwrap();
                    self.finish(
                        (0..4)
                            .filter(|&i| self.scores[i] == low)
                            .map(|i| SEATS[i])
                            .collect(),
                    );
                }
                if self.phase == Phase::Between {
                    self.status = "Hand scored".into();
                }
            }
            Mode::Spades => {
                for side in 0..2 {
                    let bid = self.bids[side] + self.bids[side + 2];
                    let tricks: i32 = [side, side + 2]
                        .iter()
                        .filter(|&&i| self.bids[i] > 0)
                        .map(|&i| self.tricks[i])
                        .sum();
                    let nil_bags: i32 = [side, side + 2]
                        .iter()
                        .filter(|&&i| self.bids[i] == 0)
                        .map(|&i| self.tricks[i])
                        .sum();
                    for i in [side, side + 2] {
                        if self.bids[i] == 0 {
                            self.scores[side] += if self.tricks[i] == 0 { 100 } else { -100 };
                        }
                    }
                    if tricks >= bid {
                        self.scores[side] += 10 * bid + tricks - bid;
                        self.bags[side] += tricks - bid;
                    } else {
                        self.scores[side] -= 10 * bid;
                    }
                    self.scores[side] += nil_bags;
                    self.bags[side] += nil_bags;
                    if self.bags[side] >= 10 {
                        self.scores[side] -= 100;
                        self.bags[side] -= 10;
                    }
                }
                if self.scores[..2]
                    .iter()
                    .any(|v| *v >= self.rules.target.unwrap())
                    && self.scores[0] != self.scores[1]
                {
                    self.finish(vec![if self.scores[0] > self.scores[1] {
                        "North–South"
                    } else {
                        "East–West"
                    }]);
                }
                if self.phase == Phase::Between {
                    self.status = "Hand scored".into();
                }
            }
            Mode::Bridge => {
                let bid = self.contract.unwrap();
                let side = bid.owner % 2;
                let tricks = self.tricks[side] + self.tricks[side + 2];
                let vulnerable = self.bridge_vulnerability()[side];
                let score = bridge_score(bid, tricks, vulnerable);
                if score >= 0 {
                    self.scores[side] += score;
                } else {
                    self.scores[1 - side] -= score;
                }
                self.status = format!(
                    "{} · {tricks} tricks · {score:+} to {}",
                    bid.label(),
                    if side == 0 {
                        "North–South"
                    } else {
                        "East–West"
                    }
                );
            }
        }
    }

    fn finish(&mut self, winners: Vec<&str>) {
        self.phase = Phase::Finished;
        self.status = if winners.len() == 1 {
            format!("{} wins", winners[0])
        } else {
            format!("Tie: {}", winners.join(", "))
        };
    }

    fn score_line(&self) -> String {
        if self.rules.mode == Mode::Hearts {
            SEATS
                .iter()
                .zip(self.scores)
                .map(|(name, score)| format!("{name} {score}"))
                .collect::<Vec<_>>()
                .join("   ·   ")
        } else {
            format!(
                "North–South {}   ·   East–West {}",
                self.scores[0], self.scores[1]
            )
        }
    }

    fn bridge_vulnerability(&self) -> [bool; 2] {
        match (self.deal - 1) % 16 {
            1 | 4 | 11 | 14 => [true, false],
            2 | 5 | 8 | 15 => [false, true],
            3 | 6 | 9 | 12 => [true, true],
            _ => [false, false],
        }
    }

    pub fn table(&self, agent: bool) -> wire::Table {
        use wire::zone::Layout::Hand;
        let viewer = if agent { self.actor() } else { 0 };
        let mut zones = Vec::new();
        let mut view = Vec::new();
        let dummy = self.declarer.map(|d| (d + 2) % 4);
        let dummy_visible = dummy.filter(|_| !self.trick.is_empty() || !self.last_trick.is_empty());
        for seat in [2, 1, 3, 0] {
            let hand = &self.hands[seat];
            let visible = viewer == seat || dummy_visible == Some(seat);
            let seat_name = format!(
                "{}{}{}",
                SEATS[seat],
                if seat == viewer { " (you)" } else { "" },
                if dummy_visible == Some(seat) {
                    " · dummy"
                } else {
                    ""
                }
            );
            let cards = if visible {
                hand.iter().map(|c| c.view()).collect()
            } else {
                hand.iter().map(|_| card::hidden()).collect()
            };
            zones.push(card::zone(
                HAND + seat as u32,
                if seat == 0 {
                    3
                } else if seat == 2 {
                    0
                } else {
                    1
                },
                seat_name.clone(),
                Hand,
                cards,
                hand.len(),
            ));
            if visible {
                view.push(format!(
                    "{seat_name} hand: {}",
                    hand.iter().map(|c| c.short()).collect::<Vec<_>>().join(" ")
                ));
            }
        }
        let active_trick = if self.trick.is_empty() {
            &self.last_trick
        } else {
            &self.trick
        };
        let label = if self.trick.is_empty() && !self.last_trick.is_empty() {
            "Last trick"
        } else {
            "Current trick"
        };
        let played = active_trick
            .iter()
            .map(|(seat, c)| format!("{} {}", SEATS[*seat], c.short()))
            .collect::<Vec<_>>()
            .join("  ·  ");
        zones.push(card::zone(
            TRICK,
            2,
            if played.is_empty() {
                label.into()
            } else {
                format!("{label}: {played}")
            },
            Hand,
            active_trick.iter().map(|(_, c)| c.view()).collect(),
            active_trick.len(),
        ));
        let trick_line = SEATS
            .iter()
            .zip(self.tricks)
            .map(|(s, n)| format!("{s} {n}"))
            .collect::<Vec<_>>()
            .join(" · ");
        let context = self
            .contract
            .map_or(String::new(), |b| format!(" · {}", b.label()));
        let mut status = match self.phase {
            Phase::Pass => format!(
                "{} passes {} more to {}",
                SEATS[self.turn],
                3 - self.passed[self.turn].len(),
                SEATS[(self.turn + self.pass_offset()) % 4]
            ),
            Phase::Bid => format!("{} to bid{context}", SEATS[self.turn]),
            Phase::Play => format!("{} to play{context}", SEATS[self.turn]),
            Phase::Between | Phase::Finished => self.status.clone(),
        };
        if self.rules.mode == Mode::Bridge {
            let vulnerability = match self.bridge_vulnerability() {
                [true, true] => "both vulnerable",
                [true, false] => "North–South vulnerable",
                [false, true] => "East–West vulnerable",
                [false, false] => "neither vulnerable",
            };
            status = format!("Board {} · {status} · {vulnerability}", self.deal);
        }
        let score = self.score_line();
        view.push(format!(
            "{status}. {label}: {played}. Tricks: {trick_line}. Score: {score}."
        ));
        wire::Table {
            zones,
            status: format!("{status}   |   {score}"),
            agent_turn: self.actor() != 0,
            finished: self.phase == Phase::Finished,
            player_view: view.join("\n"),
            events: self.events.clone(),
            ..Default::default()
        }
    }
}

fn heart_points(card: Card) -> i32 {
    if card.suit == Suit::Hearts {
        1
    } else if card.suit == Suit::Spades && card.rank == 12 {
        13
    } else {
        0
    }
}

fn bridge_score(contract: Contract, tricks: i32, vulnerable: bool) -> i32 {
    let level = i32::from(contract.level());
    let modifier = i32::from(contract.multiplier);
    let excess = tricks - 6 - level;
    if excess < 0 {
        let under = -excess;
        if modifier == 1 {
            return -under * if vulnerable { 100 } else { 50 };
        }
        let doubled = if vulnerable {
            200 + 300 * (under - 1)
        } else {
            100 + 200 * (under - 1).min(2) + 300 * (under - 3).max(0)
        };
        return -doubled * modifier / 2;
    }
    let per_trick = if contract.strain() < 2 { 20 } else { 30 };
    let contracted = (level * per_trick + if contract.strain() == 4 { 10 } else { 0 }) * modifier;
    let game = if contracted < 100 {
        50
    } else if vulnerable {
        500
    } else {
        300
    };
    let slam = match (level, vulnerable) {
        (6, false) => 500,
        (6, true) => 750,
        (7, false) => 1000,
        (7, true) => 1500,
        _ => 0,
    };
    let overtricks = excess
        * if modifier == 1 {
            per_trick
        } else {
            (if vulnerable { 200 } else { 100 }) * modifier / 2
        };
    contracted + game + slam + overtricks + if modifier > 1 { 25 * modifier } else { 0 }
}
