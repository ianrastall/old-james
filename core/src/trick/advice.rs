//! Built-in Hearts and Spades play. Each decision uses only what the acting
//! seat can see: its own hand, the bids, the cards it passed and the cards
//! played this deal.
//!
//! Card play is a Monte Carlo search. The seat deals the unseen cards to the
//! other hands many times, consistent with the suits each seat has shown out
//! of, plays each legal card in every deal, finishes the deal with a quick
//! heuristic player, and keeps the card with the best average result. Passing
//! and Spades bidding use hand estimates.

use super::{Game, Mode, Move, Phase, heart_points};
use crate::card::{Card, Suit};
use rand::{Rng, SeedableRng, rngs::StdRng, seq::SliceRandom};

/// Hidden-card deals examined for each decision.
const SAMPLES: usize = 120;
/// Chance that the quick player picks any legal card, so rollouts vary.
const NOISE: f64 = 0.1;

const HEARTS: usize = 2;
const SPADES: usize = 3;
const SUIT: [u64; 4] = [0x1FFF, 0x1FFF << 13, 0x1FFF << 26, 0x1FFF << 39];
/// Every heart and the queen of spades.
const POINTS: u64 = SUIT[HEARTS] | 1 << QUEEN_OF_SPADES;
const QUEEN_OF_SPADES: u32 = 39 + 10;

/// Bit index of a card: thirteen bits per suit, two low to ace high.
fn index(card: Card) -> u32 {
    card.suit as u32 * 13 + u32::from(card.high()) - 2
}

fn mask(cards: impl IntoIterator<Item = Card>) -> u64 {
    cards.into_iter().fold(0, |m, c| m | 1 << index(c))
}

fn suit_of(index: u32) -> usize {
    (index / 13) as usize
}

/// Rank order within a suit, 0 for the two up to 12 for the ace.
fn rank_of(index: u32) -> u32 {
    index % 13
}

fn lowest(cards: u64) -> u32 {
    cards.trailing_zeros()
}

fn highest(cards: u64) -> u32 {
    63 - cards.leading_zeros()
}

/// Cards of the same suit ranked below the given card.
fn below(index: u32) -> u64 {
    SUIT[suit_of(index)] & ((1 << index) - 1)
}

/// Cards of the same suit ranked above the given card.
fn above(index: u32) -> u64 {
    SUIT[suit_of(index)] & !((1 << (index + 1)) - 1)
}

/// The card ranked highest regardless of suit; ties go to the lower suit.
fn top_rank(cards: u64) -> u32 {
    bits(cards)
        .max_by_key(|&i| (rank_of(i), u32::MAX - i))
        .unwrap()
}

fn bottom_rank(cards: u64) -> u32 {
    bits(cards).min_by_key(|&i| (rank_of(i), i)).unwrap()
}

fn bits(mut cards: u64) -> impl Iterator<Item = u32> {
    std::iter::from_fn(move || {
        (cards != 0).then(|| {
            let i = cards.trailing_zeros();
            cards &= cards - 1;
            i
        })
    })
}

fn pick(cards: u64, rng: &mut StdRng) -> u32 {
    let n = rng.random_range(0..cards.count_ones());
    bits(cards).nth(n as usize).unwrap()
}

/// A deal in progress with every hand known, cheap to copy and play out.
#[derive(Clone, Copy)]
struct Sim {
    spades: bool,
    hands: [u64; 4],
    trick: [u32; 4],
    trick_len: usize,
    leader: usize,
    turn: usize,
    broken: bool,
    tricks_done: u32,
    points: [i32; 4],
    tricks: [i32; 4],
    bids: [i32; 4],
}

impl Sim {
    fn legal(&self) -> u64 {
        let hand = self.hands[self.turn];
        if self.trick_len > 0 {
            let suited = hand & SUIT[suit_of(self.trick[0])];
            if suited != 0 {
                return suited;
            }
            if !self.spades && self.tricks_done == 0 && hand & !POINTS != 0 {
                return hand & !POINTS;
            }
            return hand;
        }
        let closed = if self.spades { SPADES } else { HEARTS };
        if !self.spades && self.tricks_done == 0 && hand & 1 != 0 {
            1
        } else if !self.broken && hand & !SUIT[closed] != 0 {
            hand & !SUIT[closed]
        } else {
            hand
        }
    }

    /// Position in the trick of the card winning so far.
    fn winning(&self) -> usize {
        let lead = suit_of(self.trick[0]);
        (0..self.trick_len)
            .max_by_key(|&i| {
                let card = self.trick[i];
                let class = if self.spades && suit_of(card) == SPADES {
                    2
                } else {
                    u32::from(suit_of(card) == lead)
                };
                (class, rank_of(card))
            })
            .unwrap()
    }

    fn play(&mut self, card: u32) {
        let bit = 1 << card;
        debug_assert!(self.hands[self.turn] & bit != 0);
        self.hands[self.turn] &= !bit;
        self.broken |= if self.spades {
            suit_of(card) == SPADES
        } else {
            POINTS & bit != 0
        };
        self.trick[self.trick_len] = card;
        self.trick_len += 1;
        self.turn = (self.turn + 1) % 4;
        if self.trick_len < 4 {
            return;
        }
        let winner = (self.leader + self.winning()) % 4;
        self.tricks[winner] += 1;
        self.points[winner] += self.trick.iter().map(|&c| point(c)).sum::<i32>();
        self.leader = winner;
        self.turn = winner;
        self.trick_len = 0;
        self.tricks_done += 1;
    }

    fn finished(&self) -> bool {
        self.hands.iter().all(|&h| h == 0) && self.trick_len == 0
    }

    /// A quick, plausible card for the seat to act, used to finish rollouts.
    fn quick(&self, rng: &mut StdRng) -> u32 {
        let legal = self.legal();
        if legal.count_ones() == 1 {
            return lowest(legal);
        }
        if rng.random_bool(NOISE) {
            return pick(legal, rng);
        }
        self.heuristic(legal, rng)
    }

    fn heuristic(&self, legal: u64, rng: &mut StdRng) -> u32 {
        if legal.count_ones() == 1 {
            return lowest(legal);
        }
        let ducking = !self.spades || self.bids[self.turn] == 0;
        if self.trick_len == 0 {
            let suits: Vec<_> = (0..4).filter(|&s| legal & SUIT[s] != 0).collect();
            let suit = suits[rng.random_range(0..suits.len())];
            let cards = legal & SUIT[suit];
            if !ducking {
                // Cash a card nobody can beat in its suit.
                let out = self.hands.iter().fold(0, |m, h| m | h) & SUIT[suit];
                if highest(cards) == highest(out) {
                    return highest(cards);
                }
            }
            return lowest(cards);
        }
        let lead = suit_of(self.trick[0]);
        let best = self.trick[self.winning()];
        let last = self.trick_len == 3;
        let follows = legal & SUIT[lead] != 0;
        if ducking {
            if follows {
                if suit_of(best) != lead {
                    // Already trumped: shed the highest card of the suit.
                    return highest(legal);
                }
                let under = legal & below(best);
                return if under != 0 {
                    highest(under)
                } else if last {
                    highest(legal)
                } else {
                    lowest(legal)
                };
            }
            if !self.spades && legal & (1 << QUEEN_OF_SPADES) != 0 {
                return QUEEN_OF_SPADES;
            }
            if !self.spades && legal & SUIT[HEARTS] != 0 {
                return highest(legal & SUIT[HEARTS]);
            }
            let off = legal & !SUIT[SPADES];
            return top_rank(if self.spades && off != 0 { off } else { legal });
        }
        let partner = (self.leader + self.winning()) % 4 == (self.turn + 2) % 4;
        let off_trump = legal & !SUIT[SPADES];
        let shed = || bottom_rank(if off_trump != 0 { off_trump } else { legal });
        if partner && self.bids[(self.turn + 2) % 4] != 0 {
            return if follows { lowest(legal) } else { shed() };
        }
        let beat = if suit_of(best) == lead || suit_of(best) == SPADES {
            legal & above(best)
                | if suit_of(best) != SPADES && !follows {
                    legal & SUIT[SPADES]
                } else {
                    0
                }
        } else {
            0
        };
        if beat != 0 {
            lowest(beat)
        } else if follows {
            lowest(legal)
        } else {
            shed()
        }
    }

    fn rollout(mut self, rng: &mut StdRng) -> Self {
        while !self.finished() {
            let card = self.quick(rng);
            self.play(card);
        }
        self
    }
}

fn point(index: u32) -> i32 {
    if suit_of(index) == HEARTS {
        1
    } else if index == QUEEN_OF_SPADES {
        13
    } else {
        0
    }
}

impl Game {
    /// The move the built-in player would make for the seat to act.
    pub fn advise(&self) -> Option<Move> {
        if self.rules.mode == Mode::Bridge {
            return None;
        }
        let hand = &self.hands[self.turn];
        match self.phase {
            Phase::Pass => Some(Move::Card(pass_choice(hand).id)),
            Phase::Bid => Some(Move::Bid(spades_bid(hand, &self.bids, self.turn))),
            Phase::Play => Some(Move::Card(self.search().id)),
            Phase::Between | Phase::Finished => None,
        }
    }

    fn search(&self) -> Card {
        let seat = self.turn;
        let hand = &self.hands[seat];
        let mut legal = self.legal_cards(hand);
        let constraints = self.constraints(seat);
        // Cards of one suit with only played or own cards between them are
        // interchangeable; consider only the lowest of each run.
        let gone = constraints.gone;
        legal.sort_by_key(|c| index(*c));
        legal.dedup_by(|high, low| {
            let (h, l) = (index(*high), index(*low));
            suit_of(h) == suit_of(l) && below(h) & above(l) & !gone == 0
        });
        if legal.len() == 1 {
            return legal[0];
        }
        let base = self.sim();
        let mut rng = StdRng::seed_from_u64(
            self.seed ^ u64::from(self.deal) << 32 ^ (self.played.len() as u64) << 8 ^ seat as u64,
        );
        let mut totals = vec![0.0; legal.len()];
        for _ in 0..SAMPLES {
            let mut deal = base;
            deal.hands = constraints.deal(&mut rng);
            let rollout_seed = rng.random();
            for (total, card) in totals.iter_mut().zip(&legal) {
                let mut sim = deal;
                sim.play(index(*card));
                // The same seed for every candidate keeps the comparison fair.
                let end = sim.rollout(&mut StdRng::seed_from_u64(rollout_seed));
                *total += self.utility(&end, seat);
            }
        }
        let best = (0..legal.len())
            // Ties go to the lower card.
            .max_by(|&a, &b| {
                totals[a]
                    .total_cmp(&totals[b])
                    .then(legal[b].high().cmp(&legal[a].high()))
            })
            .unwrap();
        legal[best]
    }

    fn sim(&self) -> Sim {
        let mut points = [0; 4];
        for (seat, taken) in self.taken.iter().enumerate() {
            points[seat] = taken.iter().map(|c| heart_points(*c)).sum();
        }
        let mut trick = [0; 4];
        for (slot, (_, card)) in trick.iter_mut().zip(&self.trick) {
            *slot = index(*card);
        }
        Sim {
            spades: self.rules.mode == Mode::Spades,
            hands: std::array::from_fn(|s| mask(self.hands[s].iter().copied())),
            trick,
            trick_len: self.trick.len(),
            leader: self.trick.first().map_or(self.turn, |(seat, _)| *seat),
            turn: self.turn,
            broken: self.broken,
            tricks_done: self.tricks.iter().sum::<i32>() as u32,
            points,
            tricks: self.tricks,
            bids: self.bids,
        }
    }

    /// What the seat knows about where the cards it cannot see are.
    fn constraints(&self, seat: usize) -> Constraints {
        let mut known = [0; 4];
        known[seat] = mask(self.hands[seat].iter().copied());
        let played = mask(
            self.taken
                .iter()
                .flatten()
                .chain(self.trick.iter().map(|(_, c)| c))
                .copied(),
        );
        // Passed cards stay with the receiver until they are played.
        let receiver = (seat + self.pass_offset()) % 4;
        if receiver != seat {
            known[receiver] |= mask(self.gave[seat].iter().copied()) & !played;
        }
        let mut void = [[false; 4]; 4];
        for trick in self.played.chunks(4) {
            let lead = trick[0].1.suit;
            for (s, card) in &trick[1..] {
                if card.suit != lead {
                    void[*s][lead as usize] = true;
                }
            }
        }
        let unseen = !played & !known.iter().fold(0, |m, k| m | k) & ((1 << 52) - 1);
        Constraints {
            gone: played | known[seat],
            known,
            void,
            need: std::array::from_fn(|s| self.hands[s].len() as u32 - known[s].count_ones()),
            unseen,
        }
    }

    /// Value of a finished deal to the seat; higher is better.
    fn utility(&self, end: &Sim, seat: usize) -> f64 {
        if end.spades {
            let delta = |side: usize| spades_score(&end.bids, &end.tricks, self.bags[side], side);
            return f64::from(delta(seat % 2) - delta(1 - seat % 2));
        }
        let moon = end.points.iter().position(|&p| p == 26);
        let taken = |s: usize| match moon {
            Some(m) if m == s => 0,
            Some(_) => 26,
            None => end.points[s],
        };
        let others: i32 = (0..4).filter(|&s| s != seat).map(taken).sum();
        f64::from(others) / 3.0 - f64::from(taken(seat))
    }
}

/// Change to one side's Spades score for a finished deal.
fn spades_score(bids: &[i32; 4], tricks: &[i32; 4], bags: i32, side: usize) -> i32 {
    let seats = [side, side + 2];
    let bid: i32 = seats.iter().map(|&i| bids[i]).sum();
    let mut score = 0;
    let mut taken = 0;
    let mut new_bags = 0;
    for i in seats {
        if bids[i] == 0 {
            score += if tricks[i] == 0 { 100 } else { -100 };
            new_bags += tricks[i];
        } else {
            taken += tricks[i];
        }
    }
    if taken >= bid {
        score += 10 * bid + taken - bid;
        new_bags += taken - bid;
    } else {
        score -= 10 * bid;
    }
    score += new_bags - (taken - bid).max(0);
    if bags + new_bags >= 10 {
        score -= 100;
    }
    score
}

struct Constraints {
    /// Cards played this deal and the seat's own hand.
    gone: u64,
    known: [u64; 4],
    void: [[bool; 4]; 4],
    need: [u32; 4],
    unseen: u64,
}

impl Constraints {
    /// Deals the unseen cards so each hand has its size and no card lands in a
    /// suit its holder has shown out of. Falls back to ignoring voids.
    fn deal(&self, rng: &mut StdRng) -> [u64; 4] {
        let mut cards: Vec<u32> = bits(self.unseen).collect();
        for attempt in 0..30 {
            cards.shuffle(rng);
            let respect = attempt < 29;
            let open = |s: usize, c: u32| !respect || !self.void[s][suit_of(c)];
            // Most constrained cards first.
            cards.sort_by_key(|&c| (0..4).filter(|&s| self.need[s] > 0 && open(s, c)).count());
            let mut hands = self.known;
            let mut need = self.need;
            let placed = cards.iter().all(|&c| {
                let room: u32 = (0..4).filter(|&s| open(s, c)).map(|s| need[s]).sum();
                if room == 0 {
                    return false;
                }
                let mut n = rng.random_range(0..room);
                let seat = (0..4)
                    .filter(|&s| open(s, c))
                    .find(|&s| {
                        if n < need[s] {
                            true
                        } else {
                            n -= need[s];
                            false
                        }
                    })
                    .unwrap();
                hands[seat] |= 1 << c;
                need[seat] -= 1;
                true
            });
            if placed {
                return hands;
            }
        }
        unreachable!("the last attempt ignores voids and always places every card")
    }
}

/// The card to pass: the queen of spades and the spades above it unless the
/// hand can guard them, then high hearts, then high cards in short suits.
fn pass_choice(hand: &[Card]) -> Card {
    let low_spades = hand
        .iter()
        .filter(|c| c.suit == Suit::Spades && c.high() < 12)
        .count();
    let length = |suit: Suit| hand.iter().filter(|c| c.suit == suit).count();
    let danger = |card: &Card| -> i32 {
        let high = i32::from(card.high());
        match card.suit {
            Suit::Spades if high >= 12 => {
                if low_spades >= 4 {
                    high - 10
                } else if high == 12 {
                    100
                } else {
                    60 + high
                }
            }
            Suit::Spades => high,
            Suit::Hearts => 2 * high + if high >= 11 { 12 } else { 0 },
            suit => {
                let short = length(suit);
                2 * high + if short <= 3 { 16 - 4 * short as i32 } else { 0 }
            }
        }
    };
    *hand.iter().max_by_key(|c| (danger(c), c.id)).unwrap()
}

/// Estimated tricks for a Spades hand, then the bid: nil when the hand is very
/// weak and its spades are low.
fn spades_bid(hand: &[Card], bids: &[i32; 4], seat: usize) -> u8 {
    bid_with(hand, bids, seat, BIDDING)
}

/// Nil when the estimate is below `nil`, with no spade above `nil_spade`;
/// otherwise the estimate plus `lift`, rounded.
#[derive(Clone, Copy)]
struct Bidding {
    nil: f32,
    nil_spade: u8,
    lift: f32,
}

/// In 160-game matches to 500 against the same bidding without lift, a lift
/// of 0.3 won 88, a lift of -0.3 won 58, and never bidding nil won 75.
const BIDDING: Bidding = Bidding {
    nil: 0.8,
    nil_spade: 10,
    lift: 0.3,
};

fn bid_with(hand: &[Card], bids: &[i32; 4], seat: usize, style: Bidding) -> u8 {
    let length = |suit: Suit| hand.iter().filter(|c| c.suit == suit).count();
    let spades = length(Suit::Spades);
    let mut tricks = 0.0;
    for card in hand {
        let n = length(card.suit);
        tricks += match (card.suit, card.high()) {
            (Suit::Spades, 14) => 1.0,
            (Suit::Spades, 13) if n >= 2 => 1.0,
            (Suit::Spades, 13) => 0.4,
            (Suit::Spades, 12) if n >= 3 => 0.8,
            (Suit::Spades, 11) if n >= 4 => 0.5,
            (_, 14) if n <= 5 => 1.0,
            (_, 14) => 0.6,
            (_, 13) if (2..=4).contains(&n) => 0.8,
            (_, 13) if n == 5 => 0.4,
            (_, 12) if (3..=4).contains(&n) => 0.3,
            _ => 0.0,
        };
    }
    tricks += spades.saturating_sub(4) as f32 * 0.9;
    // Short side suits let spades ruff.
    let spare = spades.saturating_sub(2) as f32;
    let ruffs: f32 = [Suit::Clubs, Suit::Diamonds, Suit::Hearts]
        .iter()
        .map(|&s| match length(s) {
            0 => 1.0,
            1 => 0.6,
            2 => 0.2,
            _ => 0.0,
        })
        .sum();
    tricks += ruffs.min(spare) * 0.8;
    let high_spade = hand
        .iter()
        .filter(|c| c.suit == Suit::Spades)
        .map(|c| c.high())
        .max()
        .unwrap_or(0);
    let partner = bids[(seat + 2) % 4];
    if tricks < style.nil && high_spade <= style.nil_spade && spades <= 3 && partner != 0 {
        return 0;
    }
    let others: i32 = bids.iter().filter(|&&b| b > 0).sum();
    let bid = ((tricks + style.lift).round() as i32).max(1);
    // Keep the table total sensible.
    bid.min((13 - others).max(1)) as u8
}

impl Constraints {
    #[cfg(test)]
    fn hand_sizes_hold(&self, hands: &[u64; 4]) -> bool {
        (0..4).all(|s| {
            (hands[s] & !self.known[s]).count_ones() == self.need[s]
                && hands[s] & self.known[s] == self.known[s]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trick::Rules;
    use std::time::Instant;

    const C: Suit = Suit::Clubs;
    const D: Suit = Suit::Diamonds;
    const H: Suit = Suit::Hearts;
    const S: Suit = Suit::Spades;

    fn cards(list: &[(Suit, u8)]) -> Vec<Card> {
        list.iter()
            .enumerate()
            .map(|(i, &(suit, rank))| Card {
                id: 100 + i as u32,
                suit,
                rank,
            })
            .collect()
    }

    fn game(mode: Mode) -> Game {
        Game::new(
            Rules {
                mode,
                target: Some(if mode == Mode::Hearts { 100 } else { 500 }),
            },
            7,
        )
    }

    /// A deal in progress. Cards in no hand and not in the current trick count
    /// as taken in earlier tricks, spread around the table.
    fn position(mode: Mode, hands: [&[(Suit, u8)]; 4], trick: &[(usize, Suit, u8)]) -> Game {
        let mut g = game(mode);
        g.phase = Phase::Play;
        g.broken = true;
        g.hands = hands.map(cards);
        g.trick = trick
            .iter()
            .enumerate()
            .map(|(i, &(seat, suit, rank))| {
                (
                    seat,
                    Card {
                        id: 1 + i as u32,
                        suit,
                        rank,
                    },
                )
            })
            .collect();
        g.played = g.trick.clone();
        let held: Vec<(Suit, u8)> = g
            .hands
            .iter()
            .flatten()
            .chain(g.trick.iter().map(|(_, c)| c))
            .map(|c| (c.suit, c.rank))
            .collect();
        g.taken = Default::default();
        let mut n = 0;
        for suit in Suit::ALL {
            for rank in 1..=13 {
                if !held.contains(&(suit, rank)) {
                    g.taken[n % 4].push(Card {
                        id: 60 + n as u32,
                        suit,
                        rank,
                    });
                    n += 1;
                }
            }
        }
        assert_eq!(n % 4, 0, "earlier tricks must hold whole tricks");
        let done = n as i32 / 4;
        g.tricks = [done / 4 + done % 4, done / 4, done / 4, done / 4];
        g.turn = (trick.first().map_or(0, |t| t.0) + trick.len()) % 4;
        g
    }

    #[test]
    fn card_bits_round_trip() {
        let ace = Card {
            id: 1,
            suit: C,
            rank: 1,
        };
        let two = Card {
            id: 2,
            suit: C,
            rank: 2,
        };
        let queen = Card {
            id: 3,
            suit: S,
            rank: 12,
        };
        assert_eq!(index(two), 0);
        assert_eq!(index(ace), 12);
        assert_eq!(index(queen), QUEEN_OF_SPADES);
        assert_eq!(below(index(ace)) & SUIT[0], 0x0FFF);
    }

    #[test]
    fn passes_an_unguarded_queen_of_spades_first() {
        let hand = cards(&[(S, 12), (S, 3), (H, 1), (C, 10), (D, 4)]);
        assert_eq!(pass_choice(&hand).rank, 12);
        let guarded = cards(&[(S, 12), (S, 2), (S, 3), (S, 4), (S, 5), (H, 1), (H, 13)]);
        assert_eq!(pass_choice(&guarded).suit, H);
    }

    #[test]
    fn bids_strong_and_weak_spades_hands() {
        let strong = cards(&[
            (S, 1),
            (S, 13),
            (S, 12),
            (S, 10),
            (S, 7),
            (H, 1),
            (H, 13),
            (C, 1),
            (C, 5),
            (D, 1),
            (D, 3),
            (D, 4),
            (H, 2),
        ]);
        assert!(spades_bid(&strong, &[-1; 4], 0) >= 7);
        let weak = cards(&[
            (S, 2),
            (S, 5),
            (H, 3),
            (H, 6),
            (H, 9),
            (C, 2),
            (C, 4),
            (C, 7),
            (C, 9),
            (D, 2),
            (D, 5),
            (D, 8),
            (D, 10),
        ]);
        assert_eq!(spades_bid(&weak, &[-1; 4], 0), 0);
        // Never nil beside a nil partner.
        assert!(spades_bid(&weak, &[-1, -1, 0, -1], 0) >= 1);
    }

    #[test]
    fn hidden_deals_respect_hand_sizes_known_cards_and_voids() {
        let mut g = game(Mode::Hearts);
        let mut rng = StdRng::seed_from_u64(3);
        // Play out about half the deal using the advice itself.
        while g.phase != Phase::Play || g.played.len() < 22 {
            let m = g.advise().unwrap_or(Move::Next);
            g.apply(m);
        }
        let seat = g.turn;
        let c = g.constraints(seat);
        for _ in 0..200 {
            let hands = c.deal(&mut rng);
            assert!(c.hand_sizes_hold(&hands));
            for (dealt, hand) in hands.iter().zip(&g.hands) {
                assert_eq!(dealt.count_ones() as usize, hand.len());
            }
            assert_eq!(hands[seat], mask(g.hands[seat].iter().copied()));
        }
    }

    #[test]
    fn ducks_under_the_queen_of_spades() {
        // West led the queen of spades; South holds the king and the four.
        let g = position(
            Mode::Hearts,
            [
                &[(S, 13), (S, 4), (H, 9), (D, 7)],
                &[(C, 3), (C, 5), (D, 2)],
                &[(S, 9), (C, 8), (H, 2), (H, 5)],
                &[(S, 2), (D, 13), (C, 12), (H, 11)],
            ],
            &[(1, S, 12)],
        );
        assert_eq!(g.turn, 2);
        let mut g = g;
        // North and East duck first.
        for card in [(S, 9), (S, 2)] {
            let id = g.hands[g.turn]
                .iter()
                .find(|c| (c.suit, c.rank) == card)
                .unwrap()
                .id;
            g.apply(Move::Card(id));
        }
        assert_eq!(g.turn, 0);
        let four = g.hands[0][1].id;
        assert_eq!(g.advise(), Some(Move::Card(four)));
    }

    #[test]
    fn spades_partner_does_not_overtake_a_winning_partner() {
        // West led a low heart, North won with the ace, East followed.
        let mut g = position(
            Mode::Spades,
            [
                &[(H, 13), (H, 5), (C, 9)],
                &[(D, 3), (C, 4)],
                &[(D, 9), (C, 2)],
                &[(C, 10), (D, 12)],
            ],
            &[(1, H, 2), (2, H, 1), (3, H, 3)],
        );
        g.bids = [3, 3, 3, 3];
        assert_eq!(g.turn, 0);
        let five = g.hands[0][1].id;
        assert_eq!(g.advise(), Some(Move::Card(five)));
    }

    /// Plays whole games. The advised seats are `team`; the rest use `other`.
    fn hearts_match(games: u64, other: impl Fn(&Game, &mut StdRng) -> Move) -> [i32; 4] {
        let mut rng = StdRng::seed_from_u64(5);
        let mut totals = [0; 4];
        for seed in 0..games {
            let mut g = Game::new(
                Rules {
                    mode: Mode::Hearts,
                    target: Some(100),
                },
                seed,
            );
            while g.phase != Phase::Finished {
                let pick = match g.phase {
                    Phase::Between => Move::Next,
                    _ if g.turn == 0 => g.advise().unwrap(),
                    _ => other(&g, &mut rng),
                };
                assert!(g.choices().iter().any(|c| c.op == pick));
                g.apply(pick);
            }
            for (total, score) in totals.iter_mut().zip(g.scores) {
                *total += score;
            }
        }
        totals
    }

    fn random(g: &Game, rng: &mut StdRng) -> Move {
        let choices = g.choices();
        choices[rng.random_range(0..choices.len())].op
    }

    #[test]
    fn hearts_beats_random_players() {
        let totals = hearts_match(6, random);
        println!("hearts against random play: {totals:?}");
        let others = (totals[1] + totals[2] + totals[3]) / 3;
        assert!(totals[0] * 3 < others, "{totals:?}");
    }

    fn spades_match(games: u64, other: impl Fn(&Game, &mut StdRng) -> Move) -> (u32, u32) {
        let mut rng = StdRng::seed_from_u64(5);
        let mut won = 0;
        for seed in 0..games {
            let mut g = Game::new(
                Rules {
                    mode: Mode::Spades,
                    target: Some(500),
                },
                seed,
            );
            let mut deals = 0;
            while g.phase != Phase::Finished && deals < 60 {
                let pick = match g.phase {
                    Phase::Between => {
                        deals += 1;
                        Move::Next
                    }
                    _ if g.turn.is_multiple_of(2) => g.advise().unwrap(),
                    _ => other(&g, &mut rng),
                };
                g.apply(pick);
            }
            won += u32::from(g.scores[0] > g.scores[1]);
        }
        (won, games as u32)
    }

    #[test]
    fn spades_beats_random_players() {
        let (won, games) = spades_match(6, |g, rng| {
            if g.phase == Phase::Bid {
                // A random bid would make the opponents lose by themselves.
                Move::Bid(spades_bid(&g.hands[g.turn], &g.bids, g.turn))
            } else {
                random(g, rng)
            }
        });
        println!("spades against random card play: won {won}/{games}");
        assert_eq!(won, games);
    }

    /// The quick rollout player, without noise, as a benchmark opponent.
    fn heuristic(g: &Game, _: &mut StdRng) -> Move {
        match g.phase {
            Phase::Pass | Phase::Bid => g.advise().unwrap(),
            _ => {
                let sim = g.sim();
                let card = sim.heuristic(sim.legal(), &mut StdRng::seed_from_u64(0));
                let card = g.hands[g.turn].iter().find(|c| index(**c) == card).unwrap();
                Move::Card(card.id)
            }
        }
    }

    #[test]
    #[ignore = "benchmark; run with --ignored --nocapture"]
    fn strength_against_heuristic_players() {
        let hearts = hearts_match(40, heuristic);
        let others = f64::from(hearts[1] + hearts[2] + hearts[3]) / 3.0;
        println!(
            "hearts vs heuristic: search {} vs others {others:.0} (lower wins)",
            hearts[0]
        );
        let (won, games) = spades_match(60, heuristic);
        println!("spades vs heuristic: search side won {won}/{games}");
    }

    #[test]
    #[ignore = "benchmark; run with --ignored --nocapture"]
    fn spades_bids_in_self_play() {
        let (mut made, mut sides, mut nil_made, mut nils, mut bags) = (0, 0, 0, 0, 0);
        for seed in 0..300 {
            let mut g = game(Mode::Spades);
            g.seed = seed;
            g.deal = 0;
            g.apply(Move::Next);
            while g.phase != Phase::Between && g.phase != Phase::Finished {
                g.apply(g.advise().unwrap());
            }
            for side in 0..2 {
                let seats = [side, side + 2];
                let bid: i32 = seats.iter().map(|&i| g.bids[i]).sum();
                let taken: i32 = seats
                    .iter()
                    .filter(|&&i| g.bids[i] > 0)
                    .map(|&i| g.tricks[i])
                    .sum();
                sides += 1;
                if taken >= bid {
                    made += 1;
                    bags += taken - bid;
                }
                for i in seats.into_iter().filter(|&i| g.bids[i] == 0) {
                    nils += 1;
                    nil_made += i32::from(g.tricks[i] == 0);
                }
            }
        }
        println!(
            "spades self-play: contracts made {made}/{sides}, average bags {:.2}, nil made {nil_made}/{nils}",
            f64::from(bags) / f64::from(made)
        );
    }

    #[test]
    fn decisions_are_quick_and_repeatable() {
        let mut g = game(Mode::Spades);
        let mut slowest = 0.0f64;
        while g.phase != Phase::Between {
            let start = Instant::now();
            let m = g.advise().unwrap();
            slowest = slowest.max(start.elapsed().as_secs_f64());
            assert_eq!(g.advise(), Some(m));
            g.apply(m);
        }
        println!("slowest Spades decision: {:.0} ms", slowest * 1000.0);
        assert!(slowest < 0.5);
    }
}
