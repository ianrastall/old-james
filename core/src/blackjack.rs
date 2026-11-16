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
    bet: u32,
}

impl Rules {
    pub fn validate(&self) -> Result<(), String> {
        if (2..=1000).contains(&self.bet)
            && self.bet % 2 == 0
            && (self.bet..=100_000).contains(&self.stack)
        {
            Ok(())
        } else {
            Err("Invalid Blackjack play-chip balance or bet.".into())
        }
    }
}

#[derive(Clone, Copy)]
pub enum Move {
    Deal,
    Hit,
    Stand,
    Double,
    Split,
}

struct Hand {
    cards: Vec<Card>,
    bet: u64,
    outcome: Option<&'static str>,
}

pub struct Game {
    seed: u64,
    round: u64,
    chips: u64,
    bet: u64,
    stock: Vec<Card>,
    dealer: Vec<Card>,
    hands: Vec<Hand>,
    active: Option<usize>,
    events: Vec<String>,
}

impl Game {
    pub fn new(rules: Rules, seed: u64) -> Self {
        Self {
            seed,
            round: 0,
            chips: u64::from(rules.stack),
            bet: u64::from(rules.bet),
            stock: Vec::new(),
            dealer: Vec::new(),
            hands: Vec::new(),
            active: None,
            events: Vec::new(),
        }
    }

    pub fn choices(&self) -> Vec<Choice<Move>> {
        let choice = |action, label| Choice::new(action, label, 0, 0, 0);
        let Some(index) = self.active else {
            return if self.chips >= self.bet {
                vec![choice(
                    Move::Deal,
                    format!("Deal · {} play chips", self.bet),
                )]
            } else {
                vec![]
            };
        };
        let hand = &self.hands[index];
        let mut choices = vec![
            choice(Move::Hit, "Hit".into()),
            choice(Move::Stand, "Stand".into()),
        ];
        if hand.cards.len() == 2 && self.chips >= hand.bet {
            choices.push(choice(Move::Double, "Double down".into()));
            if self.hands.len() == 1 && hand.cards[0].rank == hand.cards[1].rank {
                choices.push(choice(Move::Split, "Split pair".into()));
            }
        }
        choices
    }

    pub fn apply(&mut self, action: Move) {
        if matches!(action, Move::Deal) {
            self.deal();
            return;
        }
        let index = self.active.unwrap();
        match action {
            Move::Hit => {
                self.hit(index);
                if value(&self.hands[index].cards).0 >= 21 {
                    self.finish_hand(index);
                }
            }
            Move::Stand => self.finish_hand(index),
            Move::Double => {
                let hand = &mut self.hands[index];
                self.chips -= hand.bet;
                hand.bet *= 2;
                self.events.push(format!("Hand {} doubles down", index + 1));
                self.hit(index);
                self.finish_hand(index);
            }
            Move::Split => {
                let hand = &mut self.hands[index];
                let card = hand.cards.pop().unwrap();
                self.chips -= hand.bet;
                let bet = hand.bet;
                self.hands.push(Hand {
                    cards: vec![card],
                    bet,
                    outcome: None,
                });
                self.events.push("You split the pair".into());
                self.hit(index);
                if card.rank == 1 || value(&self.hands[index].cards).0 == 21 {
                    self.finish_hand(index);
                }
            }
            Move::Deal => unreachable!(),
        }
    }

    fn deal(&mut self) {
        self.round = self.round.wrapping_add(1);
        self.stock = card::deck(&Suit::ALL, 1, self.seed.wrapping_add(self.round));
        self.events.clear();
        self.chips -= self.bet;
        let mut cards = Vec::new();
        self.dealer.clear();
        for _ in 0..2 {
            cards.push(self.stock.pop().unwrap());
            self.dealer.push(self.stock.pop().unwrap());
        }
        self.hands = vec![Hand {
            cards,
            bet: self.bet,
            outcome: None,
        }];
        self.active = Some(0);
        self.events
            .push(format!("Dealer shows {}", self.dealer[0].short()));
        if natural(&self.dealer) || natural(&self.hands[0].cards) {
            self.settle();
        }
    }

    fn hit(&mut self, index: usize) {
        let card = self.stock.pop().unwrap();
        self.hands[index].cards.push(card);
        self.events
            .push(format!("Hand {} draws {}", index + 1, card.short()));
    }

    fn finish_hand(&mut self, index: usize) {
        if index + 1 < self.hands.len() {
            let next = index + 1;
            self.active = Some(next);
            self.hit(next);
            let cards = &self.hands[next].cards;
            if cards[0].rank != 1 && value(cards).0 < 21 {
                return;
            }
        }
        self.settle();
    }

    fn settle(&mut self) {
        self.active = None;
        let player_natural = self.hands.len() == 1 && natural(&self.hands[0].cards);
        let dealer_natural = natural(&self.dealer);
        self.events
            .push(format!("Dealer reveals {}", self.dealer[1].short()));
        if !player_natural
            && !dealer_natural
            && self.hands.iter().any(|hand| value(&hand.cards).0 <= 21)
        {
            while value(&self.dealer).0 < 17 {
                let card = self.stock.pop().unwrap();
                self.dealer.push(card);
                self.events.push(format!("Dealer draws {}", card.short()));
            }
        }
        let dealer_total = value(&self.dealer).0;
        for (index, hand) in self.hands.iter_mut().enumerate() {
            let total = value(&hand.cards).0;
            let (description, returned) = if total > 21 {
                ("busts", 0)
            } else if player_natural && dealer_natural {
                ("pushes · both have blackjack", hand.bet)
            } else if dealer_natural {
                ("loses to dealer blackjack", 0)
            } else if player_natural {
                ("wins with blackjack", hand.bet + hand.bet * 3 / 2)
            } else if dealer_total > 21 || total > dealer_total {
                ("wins", hand.bet * 2)
            } else if total == dealer_total {
                ("pushes", hand.bet)
            } else {
                ("loses", 0)
            };
            hand.outcome = Some(description);
            self.chips += returned;
            self.events
                .push(format!("Hand {} {description}", index + 1));
        }
    }

    pub fn table(&self) -> wire::Table {
        use wire::zone::Layout::Hand;
        let dealer = self
            .dealer
            .iter()
            .enumerate()
            .map(|(index, card)| {
                if index == 1 && self.active.is_some() {
                    card::hidden()
                } else {
                    card.view()
                }
            })
            .collect();
        let label = if self.dealer.is_empty() {
            "Dealer".into()
        } else if self.active.is_some() {
            format!("Dealer shows {}", total_text(&self.dealer[..1]))
        } else {
            format!("Dealer · {}", total_text(&self.dealer))
        };
        let mut zones = vec![card::zone(1, 0, label, Hand, dealer, self.dealer.len())];
        for (index, hand) in self.hands.iter().enumerate() {
            let hand_status = hand
                .outcome
                .or_else(|| (self.active == Some(index)).then_some("to play"));
            let suffix = hand_status.map_or_else(String::new, |status| format!(" · {status}"));
            zones.push(card::zone(
                2 + index as u32,
                1,
                format!(
                    "Hand {} · {} · {} chips{suffix}",
                    index + 1,
                    total_text(&hand.cards),
                    hand.bet
                ),
                Hand,
                hand.cards.iter().map(|c| c.view()).collect(),
                hand.cards.len(),
            ));
        }
        let finished = self.active.is_none() && self.chips < self.bet;
        let status = if finished {
            format!("{} play chips · {} needed to deal", self.chips, self.bet)
        } else {
            format!("{} play chips", self.chips)
        };
        wire::Table {
            zones,
            status,
            finished,
            events: self.events.clone(),
            ..Default::default()
        }
    }
}

fn value(cards: &[Card]) -> (u8, bool) {
    let total: u8 = cards.iter().map(|card| card.rank.min(10)).sum();
    let soft = total <= 11 && cards.iter().any(|card| card.rank == 1);
    (total + if soft { 10 } else { 0 }, soft)
}

fn natural(cards: &[Card]) -> bool {
    cards.len() == 2 && value(cards).0 == 21
}

fn total_text(cards: &[Card]) -> String {
    let (total, soft) = value(cards);
    if total > 21 {
        format!("{total} · bust")
    } else if soft {
        format!("soft {total}")
    } else {
        total.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cards(ranks: &[u8]) -> Vec<Card> {
        ranks
            .iter()
            .enumerate()
            .map(|(i, &rank)| Card {
                id: i as u32 + 1,
                suit: Suit::Clubs,
                rank,
            })
            .collect()
    }

    fn round(player: &[u8], dealer: &[u8], draws: &[u8]) -> Game {
        let mut game = Game::new(
            Rules {
                stack: 100,
                bet: 10,
            },
            0,
        );
        game.hands.push(Hand {
            cards: cards(player),
            bet: 10,
            outcome: None,
        });
        game.dealer = cards(dealer);
        game.stock = cards(draws);
        game.stock.reverse();
        game.active = Some(0);
        game.chips = 90;
        game
    }

    #[test]
    fn dealer_stands_on_soft_seventeen_and_push_returns_the_bet() {
        let mut game = round(&[10, 7], &[1, 6], &[4]);
        assert_eq!(value(&game.dealer), (17, true));
        let view = game.table();
        assert!(view.zones[0].cards[1].hidden);
        assert_eq!(view.zones[0].cards[1].id, 0);
        game.apply(Move::Stand);
        assert_eq!(game.stock.len(), 1);
        assert_eq!(game.chips, 100);
        assert_eq!(game.hands[0].outcome, Some("pushes"));
        assert!(!game.table().zones[0].cards[1].hidden);
    }

    #[test]
    fn blackjack_pays_three_to_two_and_opposing_naturals_push() {
        let mut game = round(&[1, 12], &[1, 9], &[]);
        game.settle();
        assert_eq!(game.chips, 115);
        let mut game = round(&[1, 12], &[1, 10], &[]);
        game.settle();
        assert_eq!(game.chips, 100);
        let mut game = round(&[9, 7], &[1, 10], &[]);
        game.settle();
        assert_eq!(game.chips, 90);
    }

    #[test]
    fn split_aces_get_one_card_each_and_a_split_21_pays_even_money() {
        let mut game = round(&[1, 1], &[10, 9], &[13, 9, 2]);
        game.apply(Move::Split);
        assert_eq!(game.active, None);
        assert_eq!(game.chips, 120);
        assert_eq!(game.hands[0].outcome, Some("wins"));
        assert_eq!(game.hands[1].outcome, Some("wins"));
        assert_eq!(game.stock.len(), 1);
    }

    #[test]
    fn double_down_stops_after_one_draw_and_uses_hard_ace_when_needed() {
        let mut game = round(&[1, 7], &[10, 7], &[10]);
        game.apply(Move::Double);
        assert_eq!(game.active, None);
        assert_eq!(game.hands[0].cards.len(), 3);
        assert_eq!(value(&game.hands[0].cards), (18, false));
        assert_eq!(game.hands[0].bet, 20);
        assert_eq!(game.chips, 120);
    }
}
