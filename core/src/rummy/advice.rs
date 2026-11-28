//! Built-in Rummy and Gin Rummy play. Each decision uses only what the acting
//! seat can see: its own hand, the discard pile, the melds on the table and
//! the cards the other player has taken from or thrown to the discard pile.
//!
//! A hand is judged by its expected deadwood after the next draw: every unseen
//! card is added in turn, the best discard is made, and the results are
//! averaged. Discards that the other player has shown interest in are avoided.

use super::{Game, Mode, Move, Phase, best, fits, select, sort, value};
use crate::card::Card;

/// Expected-deadwood gain needed before taking the discard reveals a want.
const TAKE_MARGIN: f64 = 1.0;

fn without(cards: &[Card], card: Card) -> Vec<Card> {
    cards.iter().copied().filter(|c| *c != card).collect()
}

fn with(cards: &[Card], card: Card) -> Vec<Card> {
    let mut all = cards.to_vec();
    all.push(card);
    sort(&mut all);
    all
}

impl Game {
    /// The move the built-in player would make for the seat to act.
    pub fn advise(&self) -> Option<Move> {
        let choices: Vec<Move> = self.choices().into_iter().map(|c| c.op).collect();
        match self.phase {
            Phase::Between | Phase::Finished => None,
            Phase::Offer | Phase::Draw => Some(self.draw_choice(&choices)),
            Phase::Play => self.play_choice(&choices),
        }
    }

    /// Melds on the table that cards can be laid off against.
    fn table_melds(&self) -> Vec<Vec<Card>> {
        match self.rules.mode {
            Mode::Gin => Vec::new(),
            Mode::Basic => self.groups.iter().map(|(_, g)| g.clone()).collect(),
        }
    }

    fn deadwood(&self, cards: &[Card]) -> u32 {
        best(cards, &self.table_melds()).0
    }

    /// Cards the other player is known to hold: discards it took and has not
    /// put back down.
    fn known(&self, seat: usize) -> Vec<Card> {
        let down: Vec<Card> = self
            .discard
            .iter()
            .chain(self.groups.iter().flat_map(|(_, g)| g))
            .chain(&self.hands[1 - seat])
            .copied()
            .collect();
        self.picked[seat]
            .iter()
            .copied()
            .filter(|c| !down.contains(c))
            .collect()
    }

    /// Cards the seat cannot see: in the stock or the other player's hand.
    fn unseen(&self, seat: usize) -> Vec<Card> {
        let mut seen: Vec<Card> = self.hands[seat]
            .iter()
            .chain(&self.discard)
            .chain(self.groups.iter().flat_map(|(_, g)| g))
            .copied()
            .collect();
        seen.extend(self.known(1 - seat));
        crate::card::deck(&crate::card::Suit::ALL, 1, 0)
            .into_iter()
            .filter(|c| !seen.iter().any(|s| s.suit == c.suit && s.rank == c.rank))
            .collect()
    }

    /// Lowest deadwood after adding a card and making the best discard.
    fn after_discard(&self, hand: &[Card], melds: &[Vec<Card>]) -> u32 {
        let (deadwood, masks) = best(hand, melds);
        let used = masks.iter().fold(0u16, |m, g| m | g);
        // Discarding the highest card outside the melds is best, or close to it.
        let drop = (0..hand.len())
            .filter(|i| used & (1 << i) == 0)
            .map(|i| value(hand[i]))
            .max()
            .unwrap_or(0);
        deadwood.saturating_sub(drop)
    }

    /// Expected deadwood of a ten-card hand after one more draw and discard.
    fn outlook(&self, hand: &[Card], unseen: &[Card], melds: &[Vec<Card>]) -> f64 {
        if unseen.is_empty() {
            return f64::from(best(hand, melds).0);
        }
        let total: u32 = unseen
            .iter()
            .map(|&u| self.after_discard(&with(hand, u), melds))
            .sum();
        f64::from(total) / unseen.len() as f64
    }

    /// Extra cost of discarding a card the other player may want.
    fn danger(&self, card: Card) -> f64 {
        let other = 1 - self.turn;
        let mut risk: f64 = 0.0;
        for k in self.known(other) {
            let gap = (i32::from(k.rank) - i32::from(card.rank)).abs();
            risk += if k.rank == card.rank || k.suit == card.suit && gap == 1 {
                4.0
            } else if k.suit == card.suit && gap == 2 {
                2.0
            } else {
                0.0
            };
        }
        if self.tossed[other].iter().any(|t| t.rank == card.rank) {
            risk -= 1.0;
        }
        if self.groups.iter().any(|(_, g)| fits(card, g)) {
            risk += 3.0;
        }
        risk.clamp(0.0, 8.0)
    }

    fn draw_choice(&self, choices: &[Move]) -> Move {
        let skip = if self.phase == Phase::Offer {
            Move::Pass
        } else {
            Move::Draw
        };
        let (Some(&top), true) = (self.discard.last(), choices.contains(&Move::Take)) else {
            return skip;
        };
        let hand = &self.hands[self.turn];
        let melds = self.table_melds();
        let unseen = self.unseen(self.turn);
        let taken = with(hand, top);
        let take = taken
            .iter()
            .filter(|c| **c != top)
            .map(|&x| best(&without(&taken, x), &melds).0)
            .min()
            .unwrap();
        let draw = if unseen.is_empty() {
            f64::from(best(hand, &melds).0)
        } else {
            unseen
                .iter()
                .map(|&u| f64::from(self.after_discard(&with(hand, u), &melds)))
                .sum::<f64>()
                / unseen.len() as f64
        };
        if f64::from(take) + TAKE_MARGIN < draw {
            Move::Take
        } else {
            skip
        }
    }

    fn play_choice(&self, choices: &[Move]) -> Option<Move> {
        let hand = &self.hands[self.turn];
        let card = |id: u32| *hand.iter().find(|c| c.id == id).unwrap();
        // Gin: knock or go gin whenever the rules allow, with the lowest deadwood.
        let knock = choices
            .iter()
            .filter_map(|m| match m {
                Move::Knock(id) => Some(*id),
                _ => None,
            })
            .min_by_key(|&id| {
                (
                    self.deadwood(&without(hand, card(id))),
                    u32::MAX - value(card(id)),
                )
            });
        if let Some(id) = knock {
            return Some(Move::Knock(id));
        }
        // Rummy: put down melds from the best arrangement, then lay off.
        let arranged = best(hand, &[]).1;
        let melds: Vec<u16> = choices
            .iter()
            .filter_map(|m| match m {
                Move::Meld(mask) => Some(*mask),
                _ => None,
            })
            .collect();
        if let Some(&mask) = melds
            .iter()
            .find(|m| arranged.contains(m))
            .or(melds.first())
        {
            debug_assert!(select(hand, mask).len() >= 3);
            return Some(Move::Meld(mask));
        }
        if let Some(layoff) = choices.iter().find(|m| matches!(m, Move::Layoff(..))) {
            return Some(*layoff);
        }
        let table = self.table_melds();
        let unseen = self.unseen(self.turn);
        let scored = choices.iter().filter_map(|m| match m {
            Move::Discard(id) => {
                let c = card(*id);
                let rest = without(hand, c);
                Some((self.outlook(&rest, &unseen, &table) + self.danger(c), c))
            }
            _ => None,
        });
        // Ties shed the higher card.
        scored
            .min_by(|(a, x), (b, y)| {
                a.total_cmp(b)
                    .then(value(*y).cmp(&value(*x)))
                    .then(x.id.cmp(&y.id))
            })
            .map(|(_, c)| Move::Discard(c.id))
            .or(choices.first().copied())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::Suit;
    use crate::rummy::Rules;
    use rand::{Rng, SeedableRng, rngs::StdRng};
    use std::time::Instant;

    const C: Suit = Suit::Clubs;
    const D: Suit = Suit::Diamonds;
    const H: Suit = Suit::Hearts;
    const S: Suit = Suit::Spades;

    fn rules(mode: Mode) -> Rules {
        match mode {
            Mode::Gin => Rules {
                mode,
                target: 100,
                knock_limit: 10,
                gin_bonus: 25,
                undercut: 25,
            },
            Mode::Basic => Rules {
                mode,
                target: 100,
                knock_limit: 0,
                gin_bonus: 0,
                undercut: 0,
            },
        }
    }

    /// Card ids follow the unshuffled deck so they never collide.
    fn card(suit: Suit, rank: u8) -> Card {
        Card {
            id: suit as u32 * 13 + u32::from(rank),
            suit,
            rank,
        }
    }

    fn cards(list: &[(Suit, u8)]) -> Vec<Card> {
        let mut v: Vec<_> = list.iter().map(|&(s, r)| card(s, r)).collect();
        sort(&mut v);
        v
    }

    /// A deal where the built-in seat holds `hand`, `top` is on the discard
    /// pile, and the remaining cards are split between the stock and the
    /// other hand.
    fn position(mode: Mode, hand: &[(Suit, u8)], top: (Suit, u8)) -> Game {
        let mut g = Game::new(rules(mode), 3);
        let mine = cards(hand);
        let top = card(top.0, top.1);
        let mut rest: Vec<Card> = crate::card::deck(&Suit::ALL, 1, 9)
            .into_iter()
            .map(|c| card(c.suit, c.rank))
            .filter(|c| !mine.contains(c) && *c != top)
            .collect();
        let mut other: Vec<Card> = rest.drain(..10).collect();
        sort(&mut other);
        g.hands = [other, mine];
        g.stock = rest;
        g.discard = vec![top];
        g.turn = 1;
        g.phase = Phase::Draw;
        g
    }

    #[test]
    fn takes_a_discard_that_completes_a_meld() {
        let g = position(
            Mode::Gin,
            &[
                (H, 7),
                (H, 8),
                (C, 13),
                (D, 12),
                (S, 11),
                (C, 2),
                (D, 3),
                (S, 4),
                (H, 1),
                (C, 6),
            ],
            (H, 9),
        );
        assert_eq!(g.advise(), Some(Move::Take));
    }

    #[test]
    fn draws_from_stock_rather_than_take_an_unrelated_high_card() {
        let g = position(
            Mode::Gin,
            &[
                (H, 7),
                (H, 8),
                (C, 13),
                (D, 12),
                (S, 11),
                (C, 2),
                (D, 3),
                (S, 4),
                (H, 1),
                (C, 6),
            ],
            (S, 13),
        );
        assert_eq!(g.advise(), Some(Move::Draw));
    }

    /// The built-in seat holds eleven cards and must discard.
    fn to_discard(mode: Mode, hand: &[(Suit, u8)]) -> Game {
        let mut g = position(mode, &hand[..10], hand[10]);
        let extra = g.discard.pop().unwrap();
        g.hands[1].push(extra);
        sort(&mut g.hands[1]);
        g.discard = vec![g.stock.pop().unwrap()];
        g.phase = Phase::Play;
        g
    }

    #[test]
    fn discards_high_unconnected_deadwood() {
        let g = to_discard(
            Mode::Gin,
            &[
                (H, 7),
                (H, 8),
                (H, 9),
                (C, 4),
                (D, 4),
                (S, 4),
                (C, 1),
                (C, 2),
                (D, 2),
                (S, 13),
                (D, 9),
            ],
        );
        assert_eq!(g.advise(), Some(Move::Discard(card(S, 13).id)));
    }

    #[test]
    fn goes_gin_when_it_can() {
        let mut g = position(
            Mode::Gin,
            &[
                (H, 7),
                (H, 8),
                (H, 9),
                (C, 4),
                (D, 4),
                (S, 4),
                (C, 1),
                (C, 2),
                (C, 3),
                (S, 13),
            ],
            (H, 10),
        );
        assert_eq!(g.advise(), Some(Move::Take));
        g.apply(Move::Take);
        assert_eq!(g.advise(), Some(Move::Knock(card(S, 13).id)));
    }

    #[test]
    fn avoids_discarding_what_the_other_player_collects() {
        // Too much deadwood to knock; the nine and eight are the loose cards.
        let hand = [
            (S, 1),
            (S, 2),
            (S, 3),
            (S, 4),
            (C, 6),
            (D, 6),
            (H, 6),
            (C, 3),
            (D, 3),
            (C, 9),
            (S, 8),
        ];
        let g = to_discard(Mode::Gin, &hand);
        // With nothing known, shed the nine.
        assert_eq!(g.advise(), Some(Move::Discard(card(C, 9).id)));
        // The other player took the nine of hearts earlier.
        let mut g = g;
        let nine = card(H, 9);
        g.stock.retain(|c| *c != nine);
        g.discard.retain(|c| *c != nine);
        if !g.hands[0].contains(&nine) {
            let out = g.hands[0].pop().unwrap();
            g.stock.push(out);
            g.hands[0].push(nine);
        }
        g.picked[0].push(nine);
        assert_eq!(g.advise(), Some(Move::Discard(card(S, 8).id)));
    }

    #[test]
    fn rummy_melds_then_lays_off_then_discards() {
        let mut g = position(
            Mode::Basic,
            &[
                (H, 7),
                (H, 8),
                (H, 9),
                (C, 4),
                (D, 4),
                (S, 4),
                (C, 1),
                (D, 2),
                (C, 2),
                (S, 13),
            ],
            (H, 10),
        );
        assert_eq!(g.advise(), Some(Move::Take));
        g.apply(Move::Take);
        let mut melded = 0;
        loop {
            match g.advise().unwrap() {
                m @ Move::Meld(_) => {
                    melded += 1;
                    g.apply(m);
                }
                m @ Move::Layoff(..) => g.apply(m),
                Move::Discard(id) => {
                    assert_eq!(id, card(S, 13).id);
                    break;
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(melded, 2);
    }

    fn random(g: &Game, rng: &mut StdRng) -> Move {
        let choices = g.choices();
        choices[rng.random_range(0..choices.len())].op
    }

    /// Lowest deadwood now: take the discard only if it lowers deadwood, meld
    /// and lay off at once, knock as soon as allowed.
    fn greedy(g: &Game, _: &mut StdRng) -> Move {
        let choices: Vec<Move> = g.choices().into_iter().map(|c| c.op).collect();
        let hand = &g.hands[g.turn];
        match g.phase {
            Phase::Offer | Phase::Draw => {
                let skip = if g.phase == Phase::Offer {
                    Move::Pass
                } else {
                    Move::Draw
                };
                match g.discard.last() {
                    Some(&top) if choices.contains(&Move::Take) => {
                        let taken = with(hand, top);
                        let after = taken
                            .iter()
                            .filter(|c| **c != top)
                            .map(|&x| g.deadwood(&without(&taken, x)))
                            .min()
                            .unwrap();
                        if after < g.deadwood(hand) {
                            Move::Take
                        } else {
                            skip
                        }
                    }
                    _ => skip,
                }
            }
            _ => {
                if let Some(m) = choices
                    .iter()
                    .find(|m| matches!(m, Move::Knock(_) | Move::Meld(_) | Move::Layoff(..)))
                {
                    return *m;
                }
                *choices
                    .iter()
                    .min_by_key(|m| match m {
                        Move::Discard(id) => {
                            let c = *hand.iter().find(|c| c.id == *id).unwrap();
                            (g.deadwood(&without(hand, c)), u32::MAX - value(c))
                        }
                        _ => (u32::MAX, 0),
                    })
                    .unwrap()
            }
        }
    }

    /// Plays whole matches; the built-in player takes seat 1. Returns the
    /// matches it won and the points each side scored.
    fn play(
        mode: Mode,
        games: u64,
        stall_ok: bool,
        other: impl Fn(&Game, &mut StdRng) -> Move,
    ) -> (u32, u32, [u32; 2]) {
        let mut stalled = 0;
        let mut rng = StdRng::seed_from_u64(5);
        let mut won = 0;
        let mut points = [0; 2];
        for seed in 0..games {
            // Deals are seeded from the match seed plus the deal number, so
            // spread match seeds apart to keep their deals distinct.
            let mut g = Game::new(rules(mode), seed * 7919);
            for _ in 0..20_000 {
                if g.phase == Phase::Finished {
                    break;
                }
                let pick = match g.phase {
                    Phase::Between => Move::Next,
                    _ if g.turn == 1 => g.advise().unwrap(),
                    _ => other(&g, &mut rng),
                };
                assert!(
                    g.choices().iter().any(|c| c.op == pick),
                    "unavailable move {pick:?}"
                );
                g.apply(pick);
            }
            if g.phase != Phase::Finished {
                // Basic Rummy turns the stock over forever, so two players who
                // each hold a card the other needs can stall.
                assert!(stall_ok, "match {seed} did not finish");
                stalled += 1;
                continue;
            }
            won += u32::from(g.scores[1] > g.scores[0]);
            points[0] += g.scores[0];
            points[1] += g.scores[1];
        }
        (won, stalled, points)
    }

    #[test]
    fn beats_random_play_in_both_games() {
        for mode in [Mode::Gin, Mode::Basic] {
            let (won, _, points) = play(mode, 10, false, random);
            println!("{mode:?} against random play: won {won}/10, points {points:?}");
            assert_eq!(won, 10);
        }
    }

    #[test]
    #[ignore = "benchmark; run with --ignored --nocapture"]
    fn strength_against_greedy_play() {
        for mode in [Mode::Gin, Mode::Basic] {
            let games = 200;
            let (won, stalled, points) = play(mode, games, true, greedy);
            println!(
                "{mode:?} against greedy play: won {won}/{games}, stalled {stalled}, points {points:?}"
            );
        }
    }

    #[test]
    fn decisions_are_quick_and_repeatable() {
        for mode in [Mode::Gin, Mode::Basic] {
            let mut g = Game::new(rules(mode), 1);
            let mut slowest = 0.0f64;
            for _ in 0..200 {
                if matches!(g.phase, Phase::Between | Phase::Finished) {
                    break;
                }
                let start = Instant::now();
                let m = g.advise().unwrap();
                slowest = slowest.max(start.elapsed().as_secs_f64());
                assert_eq!(g.advise(), Some(m));
                g.apply(m);
            }
            println!("slowest {mode:?} decision: {:.1} ms", slowest * 1000.0);
            assert!(slowest < 0.3);
        }
    }
}
