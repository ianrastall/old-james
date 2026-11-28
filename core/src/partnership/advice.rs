//! Built-in Euchre play. Each decision uses only what the acting seat can see:
//! its own hand, the upcard, the bidding, and the cards played this deal.

use super::{Game, Mode, Move, Phase};
use crate::card::{Card, Suit};

const RANKS: [u8; 6] = [9, 10, 11, 12, 13, 1];
/// Hand value needed to make trump. In 2,000-game self-play matches, 5.5 beat
/// both 5.0 and 6.0 by more than three standard deviations.
const MAKE: f32 = 5.5;

/// The suit a card belongs to once trump is known (the left bower is trump).
fn suit_of(card: Card, trump: Suit) -> Suit {
    if card.rank == 11 && card.suit != trump && card.suit.red() == trump.red() {
        trump
    } else {
        card.suit
    }
}

fn is_trump(card: Card, trump: Suit) -> bool {
    suit_of(card, trump) == trump
}

/// Rank within the card's own suit: right bower 16, left bower 15, ace 14, down to 9.
fn power(card: Card, trump: Suit) -> u8 {
    if card.rank == 11 && is_trump(card, trump) {
        if card.suit == trump { 16 } else { 15 }
    } else {
        card.high()
    }
}

/// Trick-taking strength once a suit has been led.
fn strength(card: Card, lead: Suit, trump: Suit) -> (u8, u8) {
    let suit = suit_of(card, trump);
    let class = if suit == trump {
        2
    } else {
        u8::from(suit == lead)
    };
    (class, power(card, trump))
}

fn same(a: Card, b: Card) -> bool {
    a.suit == b.suit && a.rank == b.rank
}

/// Estimated value of a five-card hand with the given trump.
fn hand_value(hand: &[Card], trump: Suit) -> f32 {
    let trumps = hand.iter().filter(|c| is_trump(**c, trump)).count();
    let mut value = 0.0;
    for &card in hand {
        if is_trump(card, trump) {
            value += match power(card, trump) {
                16 => 3.0,
                15 => 2.6,
                14 => 2.2,
                13 => 1.8,
                12 => 1.6,
                10 => 1.4,
                _ => 1.3,
            };
        } else if card.rank == 1 {
            value += 1.0;
        } else if card.rank == 13
            && hand
                .iter()
                .any(|c| c.rank == 1 && suit_of(*c, trump) == suit_of(card, trump))
        {
            value += 0.4;
        }
    }
    if trumps >= 2 {
        let voids = Suit::ALL
            .into_iter()
            .filter(|s| *s != trump && !hand.iter().any(|c| suit_of(*c, trump) == *s))
            .count();
        value += 0.5 * voids as f32;
    }
    value
}

/// The dealer's discard after picking up the upcard. The upcard itself must be kept.
fn discard(hand: &[Card], upcard: Card, trump: Suit) -> Card {
    hand.iter()
        .copied()
        .filter(|c| c.id != upcard.id)
        .min_by_key(|&c| {
            let suit = suit_of(c, trump);
            let in_suit = hand.iter().filter(|o| suit_of(**o, trump) == suit).count();
            // Keep trump and aces; prefer a discard that leaves a void; then the lowest card.
            (suit == trump, c.rank == 1, in_suit != 1, power(c, trump))
        })
        .unwrap()
}

fn value_after_pickup(hand: &[Card], upcard: Card) -> f32 {
    let trump = upcard.suit;
    let mut six = hand.to_vec();
    six.push(upcard);
    let out = discard(&six, upcard, trump);
    six.retain(|c| c.id != out.id);
    hand_value(&six, trump)
}

/// Play alone only with the top trumps and nothing that can lose a trick.
fn goes_alone(hand: &[Card], trump: Suit) -> bool {
    let mut trumps: Vec<u8> = hand
        .iter()
        .filter(|c| is_trump(**c, trump))
        .map(|c| power(*c, trump))
        .collect();
    trumps.sort_unstable_by(|a, b| b.cmp(a));
    let top = [16, 15, 14, 13, 12]
        .iter()
        .zip(&trumps)
        .take_while(|(want, have)| want == have)
        .count();
    let side_aces = hand.iter().all(|c| is_trump(*c, trump) || c.rank == 1);
    (top >= 3 && side_aces) || (trumps.len() == 5 && top >= 2)
}

impl Game {
    /// The built-in choice for the seat to act, or `None` where this game has none.
    pub fn advise(&self) -> Option<Move> {
        if self.rules.mode != Mode::Euchre {
            return None;
        }
        let seat = self.turn;
        let hand = &self.hands[seat];
        let upcard = self.upcard?;
        Some(match self.phase {
            Phase::Order => {
                let trump = upcard.suit;
                let value = if seat == self.dealer {
                    value_after_pickup(hand, upcard)
                } else {
                    // The upcard joins the dealer's hand: a gain for the dealer's partner.
                    let up = hand_value(&[upcard], trump) * 0.5;
                    let partner = (seat + 2) % 4 == self.dealer;
                    hand_value(hand, trump) + if partner { up } else { -up }
                };
                if value >= MAKE {
                    Move::Trump(trump)
                } else {
                    Move::Pass
                }
            }
            Phase::Call => Suit::ALL
                .into_iter()
                .filter(|s| *s != upcard.suit)
                .map(|s| (s, hand_value(hand, s)))
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .filter(|(_, value)| *value >= MAKE)
                .map_or(Move::Pass, |(suit, _)| Move::Trump(suit)),
            Phase::Discard => Move::Card(discard(hand, upcard, self.trump?).id),
            Phase::Alone => Move::Alone(goes_alone(hand, self.trump?)),
            Phase::Play => Move::Card(self.choose_card(seat)),
            _ => return None,
        })
    }

    /// Cards this seat has not seen: not in its hand, not played, and not the turned-down upcard.
    fn unseen(&self, seat: usize, trump: Suit) -> Vec<Card> {
        let upcard = self.upcard.unwrap();
        let mut seen: Vec<Card> = self.hands[seat].clone();
        seen.extend(self.history.iter().map(|(_, c)| *c));
        if upcard.suit != trump {
            seen.push(upcard);
        }
        Suit::ALL
            .into_iter()
            .flat_map(|suit| RANKS.map(|rank| Card { id: 0, suit, rank }))
            .filter(|c| !seen.iter().any(|s| same(*s, *c)))
            .collect()
    }

    /// Suits each seat has shown out of this deal.
    fn voids(&self, trump: Suit) -> [Vec<Suit>; 4] {
        let mut voids: [Vec<Suit>; 4] = Default::default();
        let players = if self.alone { 3 } else { 4 };
        for trick in self.history.chunks(players) {
            let lead = suit_of(trick[0].1, trump);
            for (seat, card) in &trick[1..] {
                if suit_of(*card, trump) != lead && !voids[*seat].contains(&lead) {
                    voids[*seat].push(lead);
                }
            }
        }
        voids
    }

    fn choose_card(&self, seat: usize) -> u32 {
        let trump = self.trump.unwrap();
        let hand = &self.hands[seat];
        let legal = self.legal(hand);
        if legal.len() == 1 {
            return legal[0].id;
        }
        let unseen = self.unseen(seat, trump);
        let unseen_trumps = unseen.iter().filter(|c| is_trump(**c, trump)).count();
        let lowest = |cards: &[Card]| {
            cards
                .iter()
                .copied()
                .min_by_key(|c| (is_trump(*c, trump), power(*c, trump)))
                .unwrap()
        };
        let Some((_, first)) = self.trick.first() else {
            return self.lead(seat, &legal, &unseen, unseen_trumps).id;
        };
        let lead = suit_of(*first, trump);
        let key = |c: Card| strength(c, lead, trump);

        // Seats still to play after this one, in order.
        let players = if self.alone { 3 } else { 4 };
        let mut after = Vec::new();
        let mut next = seat;
        for _ in 0..players - self.trick.len() - 1 {
            next = self.next(next);
            after.push(next);
        }
        let opponents_after: Vec<usize> = after
            .iter()
            .copied()
            .filter(|s| s % 2 != seat % 2)
            .collect();
        let voids = self.voids(trump);
        let lead_seen = self
            .history
            .iter()
            .map(|(_, c)| *c)
            .chain(hand.iter().copied())
            .filter(|c| suit_of(*c, trump) == lead)
            .count();
        // Whether a card would hold the trick against every opponent still to play.
        let holds = |card: Card| {
            if opponents_after.is_empty() {
                return true;
            }
            if unseen
                .iter()
                .any(|u| suit_of(*u, trump) == suit_of(card, trump) && key(*u) > key(card))
            {
                return false;
            }
            if is_trump(card, trump) || unseen_trumps == 0 {
                return true;
            }
            let may_ruff = opponents_after.iter().any(|s| voids[*s].contains(&lead));
            !may_ruff && lead_seen < 4
        };

        let (winner_seat, winner_card) = self.current_winner().unwrap();
        let winners: Vec<Card> = legal
            .iter()
            .copied()
            .filter(|c| key(*c) > key(winner_card))
            .collect();
        let lowest_holding = winners
            .iter()
            .copied()
            .filter(|c| holds(*c))
            .min_by_key(|c| key(*c));

        if winner_seat % 2 == seat % 2 {
            // Partner is winning: stay out of the way unless their card is at risk
            // and this seat can make the trick certain.
            if holds(winner_card) {
                return lowest(&legal).id;
            }
            return lowest_holding.unwrap_or_else(|| lowest(&legal)).id;
        }
        if let Some(card) = lowest_holding {
            return card.id;
        }
        if let Some(card) = winners.iter().copied().min_by_key(|c| key(*c)) {
            return card.id;
        }
        lowest(&legal).id
    }

    fn lead(&self, seat: usize, legal: &[Card], unseen: &[Card], unseen_trumps: usize) -> Card {
        let trump = self.trump.unwrap();
        let maker = self.maker.unwrap();
        let boss = |c: Card| {
            !unseen.iter().any(|u| {
                suit_of(*u, trump) == suit_of(c, trump) && power(*u, trump) > power(c, trump)
            })
        };
        let mut trumps: Vec<Card> = legal
            .iter()
            .copied()
            .filter(|c| is_trump(*c, trump))
            .collect();
        trumps.sort_by_key(|c| std::cmp::Reverse(power(*c, trump)));
        let side: Vec<Card> = legal
            .iter()
            .copied()
            .filter(|c| !is_trump(*c, trump))
            .collect();
        let in_suit = |suit: Suit| legal.iter().filter(|c| suit_of(**c, trump) == suit).count();
        let played_in = |suit: Suit| {
            self.history
                .iter()
                .filter(|(_, c)| suit_of(*c, trump) == suit)
                .count()
        };

        // The maker draws trump while opponents may still hold some.
        if seat == maker
            && unseen_trumps > 0
            && !trumps.is_empty()
            && (boss(trumps[0]) || trumps.len() >= 2)
        {
            return trumps[0];
        }
        // With no trump left outside this hand, every top card is a sure trick.
        if unseen_trumps == 0
            && let Some(card) = legal.iter().copied().find(|c| boss(*c))
        {
            return card;
        }
        // A side-suit winner, from the suit least likely to be ruffed.
        if let Some(card) = side
            .iter()
            .copied()
            .filter(|c| boss(*c))
            .min_by_key(|c| played_in(suit_of(*c, trump)))
        {
            return card;
        }
        // Lead trump back to a partner who made it.
        if maker % 2 == seat % 2 && seat != maker && !trumps.is_empty() {
            return if boss(trumps[0]) {
                trumps[0]
            } else {
                *trumps.last().unwrap()
            };
        }
        // Lead low from the shortest side suit to build a void.
        if let Some(card) = side
            .iter()
            .copied()
            .min_by_key(|c| (in_suit(suit_of(*c, trump)), c.high()))
        {
            return card;
        }
        if boss(trumps[0]) {
            trumps[0]
        } else {
            *trumps.last().unwrap()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::partnership::{Rules, SEATS};
    use Suit::{Clubs as C, Diamonds as D, Hearts as H, Spades as S};
    use rand::{Rng, SeedableRng, rngs::StdRng};

    fn card(suit: Suit, rank: u8) -> Card {
        let id = suit as u32 * 13 + u32::from(rank);
        Card { id, suit, rank }
    }

    fn cards(list: &[(Suit, u8)]) -> Vec<Card> {
        list.iter().map(|(s, r)| card(*s, *r)).collect()
    }

    fn game() -> Game {
        Game::new(
            Rules {
                mode: Mode::Euchre,
                target: 10,
            },
            7,
        )
    }

    /// A deal in the play phase with hearts trump, made by `maker`, with the given trick so far.
    fn playing(
        seat: usize,
        hand: &[(Suit, u8)],
        maker: usize,
        trick: &[(usize, (Suit, u8))],
    ) -> Game {
        let mut g = game();
        g.phase = Phase::Play;
        g.trump = Some(H);
        g.maker = Some(maker);
        g.upcard = Some(card(H, 9));
        g.dealer = 3;
        g.hands[seat] = cards(hand);
        g.turn = seat;
        g.trick = trick
            .iter()
            .map(|(s, (suit, r))| (*s, card(*suit, *r)))
            .collect();
        g.history = g.trick.clone();
        g
    }

    fn chosen(g: &Game) -> Card {
        match g.advise().unwrap() {
            Move::Card(id) => *g.hands[g.turn].iter().find(|c| c.id == id).unwrap(),
            other => panic!("expected a card, got {other:?}"),
        }
    }

    #[test]
    fn orders_strong_hands_and_passes_weak_ones() {
        let mut g = game();
        g.phase = Phase::Order;
        g.dealer = 3;
        g.turn = 0;
        g.upcard = Some(card(H, 10));
        g.hands[0] = cards(&[(H, 11), (D, 11), (H, 1), (C, 9), (S, 10)]);
        assert_eq!(g.advise(), Some(Move::Trump(H)));
        g.hands[0] = cards(&[(H, 9), (C, 1), (S, 10), (D, 12), (C, 10)]);
        assert_eq!(g.advise(), Some(Move::Pass));
    }

    #[test]
    fn calls_the_best_other_suit_in_the_second_round() {
        let mut g = game();
        g.phase = Phase::Call;
        g.dealer = 3;
        g.turn = 0;
        g.upcard = Some(card(H, 10));
        g.hands[0] = cards(&[(S, 11), (C, 11), (S, 1), (S, 13), (D, 1)]);
        assert_eq!(g.advise(), Some(Move::Trump(S)));
        g.hands[0] = cards(&[(S, 9), (C, 10), (D, 12), (H, 1), (D, 10)]);
        assert_eq!(g.advise(), Some(Move::Pass));
    }

    #[test]
    fn dealer_discards_to_a_void_and_keeps_trump_and_aces() {
        let hand = cards(&[(H, 13), (H, 9), (C, 1), (C, 9), (S, 10), (H, 12)]);
        let out = discard(&hand, card(H, 12), H);
        assert!(same(out, card(S, 10)), "discarded {}", out.short());
    }

    #[test]
    fn goes_alone_only_with_a_sure_sweep() {
        assert!(goes_alone(
            &cards(&[(H, 11), (D, 11), (H, 1), (S, 1), (C, 1)]),
            H
        ));
        assert!(!goes_alone(
            &cards(&[(H, 11), (D, 11), (H, 1), (S, 1), (C, 9)]),
            H
        ));
        assert!(!goes_alone(
            &cards(&[(H, 11), (H, 1), (H, 13), (S, 1), (C, 1)]),
            H
        ));
    }

    #[test]
    fn maker_leads_the_right_bower() {
        let g = playing(0, &[(H, 11), (H, 10), (C, 1), (S, 9), (D, 10)], 0, &[]);
        assert!(same(chosen(&g), card(H, 11)));
    }

    #[test]
    fn defender_leads_a_side_ace_rather_than_trump() {
        let g = playing(0, &[(H, 10), (C, 1), (S, 9), (D, 10), (C, 10)], 1, &[]);
        assert!(same(chosen(&g), card(C, 1)));
    }

    #[test]
    fn does_not_overtake_a_partner_who_holds_the_trick() {
        // North led the ace of clubs; East follows low. South is last and should play low.
        let g = playing(
            0,
            &[(C, 13), (C, 9), (H, 11), (S, 10), (D, 10)],
            1,
            &[(2, (C, 1)), (3, (C, 10))],
        );
        assert!(same(chosen(&g), card(C, 9)));
    }

    #[test]
    fn does_not_trump_a_partner_who_holds_the_trick() {
        let g = playing(
            0,
            &[(H, 10), (S, 9), (D, 10), (D, 12)],
            1,
            &[(2, (C, 1)), (3, (C, 10))],
        );
        let pick = chosen(&g);
        assert!(!is_trump(pick, H), "trumped partner with {}", pick.short());
        assert!(same(pick, card(S, 9)));
    }

    #[test]
    fn last_seat_wins_as_cheaply_as_possible() {
        let g = playing(
            0,
            &[(H, 11), (H, 10), (S, 9), (D, 10)],
            1,
            &[(1, (C, 1)), (2, (C, 10)), (3, (C, 12))],
        );
        assert!(same(chosen(&g), card(H, 10)));
    }

    #[test]
    fn throws_the_lowest_card_when_the_trick_is_lost() {
        let g = playing(
            0,
            &[(S, 1), (S, 9), (D, 13)],
            1,
            &[(1, (H, 11)), (2, (H, 9)), (3, (H, 12))],
        );
        assert!(same(chosen(&g), card(S, 9)));
    }

    /// Plays whole games. The built-in seats are North–South; East–West use `other`.
    fn match_score(games: u32, other: impl Fn(&Game, &mut StdRng) -> Move) -> (u32, u32, u32) {
        let mut rng = StdRng::seed_from_u64(11);
        let (mut won, mut made, mut euchred) = (0, 0, 0);
        for seed in 0..u64::from(games) {
            let mut g = Game::new(
                Rules {
                    mode: Mode::Euchre,
                    target: 10,
                },
                seed,
            );
            for _ in 0..20_000 {
                if g.phase == Phase::Finished {
                    break;
                }
                let choices = g.choices();
                let pick = if g.phase == Phase::Between {
                    Move::Next
                } else if g.turn.is_multiple_of(2) {
                    g.advise().unwrap()
                } else {
                    other(&g, &mut rng)
                };
                assert!(
                    choices.iter().any(|c| c.op == pick),
                    "{} chose an unavailable move {pick:?}",
                    SEATS[g.turn]
                );
                let maker = g.maker;
                g.apply(pick);
                if (g.phase == Phase::Between || g.phase == Phase::Finished)
                    && let Some(maker) = maker.filter(|_| g.trump.is_some())
                {
                    if g.tricks[maker % 2] >= 3 {
                        made += 1;
                    } else {
                        euchred += 1;
                    }
                }
            }
            assert!(g.phase == Phase::Finished, "game {seed} did not finish");
            won += u32::from(g.scores[0] >= 10);
        }
        (won, made, euchred)
    }

    #[test]
    fn self_play_is_legal_and_makers_usually_succeed() {
        let (won, made, euchred) = match_score(200, |g, _| g.advise().unwrap());
        let rate = euchred as f32 / (made + euchred) as f32;
        println!("self-play: North–South won {won}/200, euchre rate {rate:.2}");
        assert!((60..=140).contains(&won));
        assert!(rate < 0.30, "makers euchred too often: {rate:.2}");
    }

    #[test]
    fn beats_random_play_decisively() {
        let (won, ..) = match_score(200, |g, rng| {
            let choices = g.choices();
            choices[rng.random_range(0..choices.len())].op
        });
        println!("against random play: won {won}/200");
        assert!(won >= 190);
    }
}
