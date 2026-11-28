use crate::{
    Choice,
    card::{self, Card, Suit},
    wire,
};
use serde::Deserialize;

mod advice;

pub(crate) const SEATS: [&str; 4] = ["South", "West", "North", "East"];
const HAND: u32 = 10;
const TRICK: u32 = 20;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Euchre,
    Pinochle,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    mode: Mode,
    target: i32,
}

impl Rules {
    pub fn validate(&self) -> Result<(), String> {
        if (1..=1000).contains(&self.target) {
            Ok(())
        } else {
            Err("Invalid partnership score target.".into())
        }
    }

    /// Whether the engine can choose moves for this game.
    pub fn advised(&self) -> bool {
        self.mode == Mode::Euchre
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Order,
    Call,
    Discard,
    Alone,
    Bid,
    Name,
    Send,
    Return,
    Play,
    Between,
    Finished,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Move {
    Pass,
    Trump(Suit),
    Card(u32),
    Bid(i32),
    Alone(bool),
    Next,
}

pub struct Game {
    rules: Rules,
    seed: u64,
    deal: u32,
    dealer: usize,
    turn: usize,
    phase: Phase,
    hands: [Vec<Card>; 4],
    upcard: Option<Card>,
    trump: Option<Suit>,
    maker: Option<usize>,
    alone: bool,
    passes: usize,
    out: [bool; 4],
    bid: i32,
    exchange: Vec<Card>,
    trick: Vec<(usize, Card)>,
    last: Vec<(usize, Card)>,
    // Every card played this deal, in order. Seats see the same public record.
    history: Vec<(usize, Card)>,
    tricks: [i32; 2],
    counters: [i32; 2],
    meld: [i32; 2],
    meld_cards: [Vec<Card>; 4],
    scores: [i32; 2],
    events: Vec<String>,
    result: String,
}

impl Game {
    pub fn new(rules: Rules, seed: u64) -> Self {
        let mut game = Self {
            rules,
            seed,
            deal: 0,
            dealer: 0,
            turn: 0,
            phase: Phase::Between,
            hands: Default::default(),
            upcard: None,
            trump: None,
            maker: None,
            alone: false,
            passes: 0,
            out: [false; 4],
            bid: 0,
            exchange: vec![],
            trick: vec![],
            last: vec![],
            history: vec![],
            tricks: [0; 2],
            counters: [0; 2],
            meld: [0; 2],
            meld_cards: Default::default(),
            scores: [0; 2],
            events: vec![],
            result: String::new(),
        };
        game.deal();
        game
    }

    fn deal(&mut self) {
        self.dealer = self.deal as usize % 4;
        self.turn = (self.dealer + 1) % 4;
        self.deal += 1;
        self.hands = Default::default();
        self.meld_cards = Default::default();
        self.upcard = None;
        self.trump = None;
        self.maker = None;
        self.alone = false;
        self.passes = 0;
        self.out = [false; 4];
        self.bid = 0;
        self.trick.clear();
        self.last.clear();
        self.history.clear();
        self.exchange.clear();
        self.tricks = [0; 2];
        self.counters = [0; 2];
        self.meld = [0; 2];
        self.events.clear();
        let euchre = self.rules.mode == Mode::Euchre;
        let mut deck = card::deck(
            &Suit::ALL,
            if euchre { 1 } else { 2 },
            self.seed.wrapping_add(u64::from(self.deal)),
        );
        deck.retain(|c| c.rank == 1 || c.rank >= 9);
        for _ in 0..if euchre { 5 } else { 12 } {
            for offset in 1..=4 {
                self.hands[(self.dealer + offset) % 4].push(deck.pop().unwrap());
            }
        }
        for hand in &mut self.hands {
            sort(hand);
        }
        if euchre {
            self.upcard = deck.pop();
        }
        self.phase = if euchre { Phase::Order } else { Phase::Bid };
    }

    pub fn actor(&self) -> usize {
        if matches!(self.phase, Phase::Between | Phase::Finished) {
            0
        } else {
            self.turn
        }
    }

    pub fn choices(&self) -> Vec<Choice<Move>> {
        let simple = |m, label| Choice::new(m, label, 0, 0, 0);
        let cards = |verb: &str, candidates: Vec<Card>, target| {
            candidates
                .into_iter()
                .map(|c| {
                    Choice::new(
                        Move::Card(c.id),
                        format!("{verb} {}", c.name()),
                        HAND + self.turn as u32,
                        c.id,
                        target,
                    )
                })
                .collect()
        };
        let hand = &self.hands[self.turn];
        match self.phase {
            Phase::Order => vec![
                simple(Move::Pass, "Pass".into()),
                simple(
                    Move::Trump(self.upcard.unwrap().suit),
                    format!("Order up {}", self.upcard.unwrap().name()),
                ),
            ],
            Phase::Call | Phase::Name => {
                let mut list = Vec::new();
                if self.phase == Phase::Call {
                    list.push(simple(Move::Pass, "Pass".into()));
                }
                list.extend(
                    Suit::ALL
                        .into_iter()
                        .filter(|s| self.phase == Phase::Name || *s != self.upcard.unwrap().suit)
                        .map(|s| simple(Move::Trump(s), format!("Name {} trump", s.name()))),
                );
                list
            }
            Phase::Discard => cards(
                "Discard",
                hand.iter()
                    .copied()
                    .filter(|c| c.id != self.upcard.unwrap().id)
                    .collect(),
                0,
            ),
            Phase::Alone => vec![
                simple(Move::Alone(false), "Play with partner".into()),
                simple(Move::Alone(true), "Go alone".into()),
            ],
            Phase::Bid => {
                let mut list = vec![simple(Move::Pass, "Pass".into())];
                let minimum = 25.max(self.bid + 1);
                list.extend((minimum..=60).map(|n| simple(Move::Bid(n), format!("Bid {n}"))));
                list
            }
            Phase::Send | Phase::Return => cards("Pass", hand.clone(), 0),
            Phase::Play => cards("Play", self.legal(hand), TRICK),
            Phase::Between => vec![simple(Move::Next, "Next deal".into())],
            Phase::Finished => vec![],
        }
    }

    fn next(&self, seat: usize) -> usize {
        (1..=4)
            .map(|step| (seat + step) % 4)
            .find(|&i| !self.out[i])
            .unwrap()
    }

    pub fn apply(&mut self, action: Move) {
        let seat = self.turn;
        match action {
            Move::Next => self.deal(),
            Move::Pass => {
                self.events.push(format!("{} passes", SEATS[seat]));
                if self.phase == Phase::Bid {
                    self.out[seat] = true;
                    if self.out.iter().filter(|b| **b).count() == 3 {
                        let maker = self.maker.unwrap_or_else(|| self.next(seat));
                        self.maker = Some(maker);
                        self.bid = self.bid.max(25);
                        self.turn = maker;
                        self.out = [false; 4];
                        self.phase = Phase::Name;
                        self.events
                            .push(format!("{} takes the bid at {}", SEATS[maker], self.bid));
                    } else {
                        self.turn = self.next(seat);
                    }
                } else {
                    self.passes += 1;
                    self.turn = self.next(seat);
                    if self.passes == 4 {
                        self.phase = Phase::Call;
                    }
                    if self.passes == 8 {
                        self.phase = Phase::Between;
                        self.result = "Passed out · no score".into();
                    }
                }
            }
            Move::Bid(bid) => {
                self.bid = bid;
                self.maker = Some(seat);
                self.events.push(format!("{} bids {bid}", SEATS[seat]));
                self.turn = self.next(seat);
            }
            Move::Trump(suit) => {
                self.trump = Some(suit);
                self.maker = Some(seat);
                self.events
                    .push(format!("{} names {} trump", SEATS[seat], suit.name()));
                match self.phase {
                    Phase::Order => {
                        self.turn = self.dealer;
                        self.hands[self.turn].push(self.upcard.unwrap());
                        sort(&mut self.hands[self.turn]);
                        self.phase = Phase::Discard;
                    }
                    Phase::Call => self.phase = Phase::Alone,
                    Phase::Name => {
                        self.turn = (seat + 2) % 4;
                        self.phase = Phase::Send;
                    }
                    _ => unreachable!(),
                }
            }
            Move::Alone(alone) => {
                self.alone = alone;
                self.out[(seat + 2) % 4] = alone;
                if alone {
                    self.events
                        .push(format!("{} is playing alone", SEATS[seat]));
                }
                self.turn = self.next(self.dealer);
                self.phase = Phase::Play;
            }
            Move::Card(id) => {
                let hand = &mut self.hands[seat];
                let index = hand.iter().position(|c| c.id == id).unwrap();
                let card = hand.remove(index);
                match self.phase {
                    Phase::Discard => {
                        self.events
                            .push(format!("{} exchanges the upcard", SEATS[seat]));
                        self.turn = self.maker.unwrap();
                        self.phase = Phase::Alone;
                    }
                    Phase::Send | Phase::Return => {
                        self.exchange.push(card);
                        if self.exchange.len() == 4 {
                            self.turn = (seat + 2) % 4;
                            self.hands[self.turn].append(&mut self.exchange);
                            sort(&mut self.hands[self.turn]);
                            if self.phase == Phase::Send {
                                self.phase = Phase::Return;
                            } else {
                                self.begin_pinochle_play();
                            }
                        }
                    }
                    Phase::Play => self.play(card),
                    _ => unreachable!(),
                }
            }
        }
    }

    fn begin_pinochle_play(&mut self) {
        self.meld = [0; 2];
        for (seat, hand) in self.hands.iter().enumerate() {
            let (points, cards) = meld_value(hand, self.trump.unwrap());
            self.meld[seat % 2] += points;
            self.events.push(format!(
                "{} shows {points} meld: {}",
                SEATS[seat],
                card_text(&cards)
            ));
            self.meld_cards[seat] = cards;
        }
        self.turn = self.maker.unwrap();
        self.phase = Phase::Play;
    }

    fn effective(&self, card: Card) -> Suit {
        let trump = self.trump.unwrap();
        if self.rules.mode == Mode::Euchre && card.rank == 11 && card.suit.red() == trump.red() {
            trump
        } else {
            card.suit
        }
    }

    fn strength(&self, card: Card, lead: Suit) -> (u8, u8) {
        let suit = self.effective(card);
        let trump = self.trump.unwrap();
        let rank = if self.rules.mode == Mode::Euchre {
            if card.rank == 11 && suit == trump {
                if card.suit == trump { 16 } else { 15 }
            } else {
                card.high()
            }
        } else {
            match card.rank {
                1 => 6,
                10 => 5,
                13 => 4,
                12 => 3,
                11 => 2,
                _ => 1,
            }
        };
        (
            if suit == trump {
                2
            } else {
                u8::from(suit == lead)
            },
            rank,
        )
    }

    fn current_winner(&self) -> Option<(usize, Card)> {
        let lead = self.effective(self.trick.first()?.1);
        // When copies tie, the card played first wins.
        self.trick
            .iter()
            .rev()
            .max_by_key(|(_, c)| self.strength(*c, lead))
            .copied()
    }

    fn legal(&self, hand: &[Card]) -> Vec<Card> {
        let Some((_, first)) = self.trick.first() else {
            return hand.to_vec();
        };
        let lead = self.effective(*first);
        let following: Vec<_> = hand
            .iter()
            .copied()
            .filter(|c| self.effective(*c) == lead)
            .collect();
        if self.rules.mode == Mode::Euchre {
            return if following.is_empty() {
                hand.to_vec()
            } else {
                following
            };
        }
        let trumping: Vec<_> = hand
            .iter()
            .copied()
            .filter(|c| c.suit == self.trump.unwrap())
            .collect();
        let candidates = if !following.is_empty() {
            following
        } else if !trumping.is_empty() {
            trumping
        } else {
            return hand.to_vec();
        };
        let current = self.strength(self.current_winner().unwrap().1, lead);
        let higher: Vec<_> = candidates
            .iter()
            .copied()
            .filter(|c| self.strength(*c, lead) > current)
            .collect();
        if higher.is_empty() {
            candidates
        } else {
            higher
        }
    }

    fn play(&mut self, card: Card) {
        self.events
            .push(format!("{} plays {}", SEATS[self.turn], card.short()));
        self.trick.push((self.turn, card));
        self.history.push((self.turn, card));
        self.turn = self.next(self.turn);
        if self.trick.len() != if self.alone { 3 } else { 4 } {
            return;
        }
        let winner = self.current_winner().unwrap().0;
        self.tricks[winner % 2] += 1;
        self.counters[winner % 2] += self
            .trick
            .iter()
            .filter(|(_, c)| matches!(c.rank, 1 | 10 | 13))
            .count() as i32;
        self.events
            .push(format!("{} takes the trick", SEATS[winner]));
        self.last = std::mem::take(&mut self.trick);
        self.turn = winner;
        if self.hands[winner].is_empty() {
            self.counters[winner % 2] += 1;
            self.score();
        }
    }

    fn score(&mut self) {
        let maker = self.maker.unwrap() % 2;
        let defender = 1 - maker;
        let mut points = [0; 2];
        if self.rules.mode == Mode::Euchre {
            match self.tricks[maker] {
                5 => points[maker] = if self.alone { 4 } else { 2 },
                3..=4 => points[maker] = 1,
                _ => points[defender] = 2,
            }
        } else {
            let earned = |side: usize| {
                if self.tricks[side] == 0 {
                    0
                } else {
                    self.counters[side] + self.meld[side]
                }
            };
            points[maker] = if earned(maker) < self.bid {
                -self.bid
            } else {
                earned(maker)
            };
            points[defender] = earned(defender);
        }
        for (score, points) in self.scores.iter_mut().zip(points) {
            *score += points;
        }
        self.phase = Phase::Between;
        self.result = format!(
            "Hand scored · North–South {:+} · East–West {:+}",
            points[0], points[1]
        );
        if let Some(winner) = [maker, defender]
            .into_iter()
            .find(|&i| self.scores[i] >= self.rules.target)
        {
            self.phase = Phase::Finished;
            self.result = format!(
                "{} wins",
                if winner == 0 {
                    "North–South"
                } else {
                    "East–West"
                }
            );
        }
    }

    pub fn table(&self, agent: bool) -> wire::Table {
        use wire::zone::Layout::{Hand, Pile};
        let viewer = if agent { self.actor() } else { 0 };
        let mut zones = Vec::new();
        for seat in [2, 1, 3, 0] {
            let hand = &self.hands[seat];
            let label = format!(
                "{}{}{}",
                SEATS[seat],
                if viewer == seat { " (you)" } else { "" },
                if self.phase == Phase::Play && self.out[seat] {
                    " · sitting out"
                } else {
                    ""
                }
            );
            zones.push(card::zone(
                HAND + seat as u32,
                if seat == 0 {
                    3
                } else if seat == 2 {
                    0
                } else {
                    1
                },
                label,
                Hand,
                hand.iter()
                    .map(|c| {
                        if viewer == seat {
                            c.view()
                        } else {
                            card::hidden()
                        }
                    })
                    .collect(),
                hand.len(),
            ));
        }
        let played = if self.trick.is_empty() {
            &self.last
        } else {
            &self.trick
        };
        let trick = played
            .iter()
            .map(|(seat, c)| format!("{} {}", SEATS[*seat], c.short()))
            .collect::<Vec<_>>()
            .join("  ·  ");
        let label = if self.trick.is_empty() && !self.last.is_empty() {
            "Last trick"
        } else {
            "Current trick"
        };
        if matches!(self.phase, Phase::Order | Phase::Call | Phase::Discard) {
            zones.push(card::zone(
                1,
                2,
                "Upcard".into(),
                Pile,
                vec![self.upcard.unwrap().view()],
                1,
            ));
        } else {
            zones.push(card::zone(
                TRICK,
                2,
                if trick.is_empty() {
                    label.into()
                } else {
                    format!("{label}: {trick}")
                },
                Hand,
                played.iter().map(|(_, c)| c.view()).collect(),
                played.len(),
            ));
        }
        let trump = self
            .trump
            .map_or(String::new(), |s| format!(" · {} trump", s.name()));
        let seat = SEATS[self.turn];
        let status = match self.phase {
            Phase::Order | Phase::Call => format!("{seat} to choose trump or pass"),
            Phase::Discard => format!("{seat} discards after picking up"),
            Phase::Alone => format!("{seat} chooses whether to go alone"),
            Phase::Bid => format!("{seat} to bid · high bid {}", self.bid),
            Phase::Name => format!("{seat} to name trump"),
            Phase::Send | Phase::Return => {
                format!("{seat} passes {} more to partner", 4 - self.exchange.len())
            }
            Phase::Play => format!("{seat} to play"),
            _ => self.result.clone(),
        };
        let score = format!(
            "North–South {}   ·   East–West {}",
            self.scores[0], self.scores[1]
        );
        let up = if matches!(self.phase, Phase::Order | Phase::Call | Phase::Discard) {
            self.upcard
                .map_or(String::new(), |c| format!(" Upcard {}.", c.short()))
        } else {
            String::new()
        };
        let player_view = format!(
            "You are {}. Your partner is {}. Dealer {}. Your hand: {}. {status}{trump}.{up} {score}. Tricks this deal: North–South {}, East–West {}. {label}: {trick}.",
            SEATS[viewer],
            SEATS[(viewer + 2) % 4],
            SEATS[self.dealer],
            card_text(&self.hands[viewer]),
            self.tricks[0],
            self.tricks[1]
        );
        wire::Table {
            zones,
            status: format!("{status}{trump}   |   {score}"),
            player_view,
            agent_turn: self.actor() != 0,
            finished: self.phase == Phase::Finished,
            events: self.events.clone(),
            ..Default::default()
        }
    }
}

fn sort(hand: &mut [Card]) {
    hand.sort_by_key(|c| (c.suit as u8, c.high()));
}
fn card_text(cards: &[Card]) -> String {
    cards
        .iter()
        .map(|c| c.short())
        .collect::<Vec<_>>()
        .join(" ")
}

fn meld_value(hand: &[Card], trump: Suit) -> (i32, Vec<Card>) {
    let counts: [[i32; 14]; 4] = std::array::from_fn(|i| {
        std::array::from_fn(|r| {
            hand.iter()
                .filter(|c| c.suit as usize == i && c.rank as usize == r)
                .count() as i32
        })
    });
    let mut visible = [[0; 14]; 4];
    let mut mark = |suit: Suit, rank: usize, quantity: i32| {
        visible[suit as usize][rank] = visible[suit as usize][rank].max(quantity);
    };
    let t = trump as usize;
    let runs = [1, 10, 11, 12, 13]
        .into_iter()
        .map(|rank| counts[t][rank])
        .min()
        .unwrap();
    let mut score = [0, 15, 150][runs as usize] + counts[t][9];
    mark(trump, 9, counts[t][9]);
    for rank in [1, 10, 11, 12, 13] {
        mark(trump, rank, runs);
    }
    for suit in Suit::ALL {
        let count = counts[suit as usize][12].min(counts[suit as usize][13]);
        score += if suit == trump {
            (count - runs) * 4
        } else {
            count * 2
        };
        mark(suit, 12, count);
        mark(suit, 13, count);
    }
    for (rank, points) in [(1, 10), (13, 8), (12, 6), (11, 4)] {
        let count = Suit::ALL
            .into_iter()
            .map(|s| counts[s as usize][rank])
            .min()
            .unwrap();
        score += [0, points, points * 10][count as usize];
        for suit in Suit::ALL {
            mark(suit, rank, count);
        }
    }
    let pinochles = counts[Suit::Spades as usize][12].min(counts[Suit::Diamonds as usize][11]);
    score += [0, 4, 30][pinochles as usize];
    mark(Suit::Spades, 12, pinochles);
    mark(Suit::Diamonds, 11, pinochles);
    let cards = hand
        .iter()
        .copied()
        .filter(|c| {
            let count = &mut visible[c.suit as usize][c.rank as usize];
            if *count == 0 {
                false
            } else {
                *count -= 1;
                true
            }
        })
        .collect();
    (score, cards)
}
