use crate::wire;
use rand::{SeedableRng, rngs::StdRng, seq::SliceRandom};
use serde::Deserialize;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Suit {
    Clubs,
    Diamonds,
    Hearts,
    Spades,
}

impl Suit {
    pub const ALL: [Self; 4] = [Self::Clubs, Self::Diamonds, Self::Hearts, Self::Spades];

    pub fn name(self) -> &'static str {
        ["clubs", "diamonds", "hearts", "spades"][self as usize]
    }

    pub fn symbol(self) -> &'static str {
        ["♣", "♦", "♥", "♠"][self as usize]
    }

    pub fn red(self) -> bool {
        matches!(self, Self::Diamonds | Self::Hearts)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Card {
    pub id: u32,
    pub suit: Suit,
    pub rank: u8,
}

impl Card {
    pub fn rank(self) -> &'static str {
        [
            "ace", "2", "3", "4", "5", "6", "7", "8", "9", "10", "jack", "queen", "king",
        ][usize::from(self.rank - 1)]
    }

    pub fn name(self) -> String {
        format!("{} of {}", self.rank(), self.suit.name())
    }

    pub fn short(self) -> String {
        let rank = [
            "A", "2", "3", "4", "5", "6", "7", "8", "9", "10", "J", "Q", "K",
        ][usize::from(self.rank - 1)];
        format!("{rank}{}", self.suit.symbol())
    }

    pub fn high(self) -> u8 {
        if self.rank == 1 { 14 } else { self.rank }
    }

    pub fn view(self) -> wire::Card {
        wire::Card {
            id: self.id,
            suit: self.suit.name().into(),
            rank: self.rank().into(),
            name: self.name(),
            hidden: false,
        }
    }
}

pub fn hidden() -> wire::Card {
    wire::Card {
        name: "Face-down card".into(),
        hidden: true,
        ..Default::default()
    }
}

pub fn deck(suits: &[Suit], copies: usize, seed: u64) -> Vec<Card> {
    let mut cards = Vec::new();
    for _ in 0..copies {
        for suit in suits {
            for rank in 1..=13 {
                cards.push(Card {
                    id: cards.len() as u32 + 1,
                    suit: *suit,
                    rank,
                });
            }
        }
    }
    // Saved games and numbered deals depend on this exact shuffle.
    cards.shuffle(&mut StdRng::seed_from_u64(seed));
    cards
}

pub fn zone(
    id: u32,
    row: u32,
    label: String,
    layout: wire::zone::Layout,
    cards: Vec<wire::Card>,
    count: usize,
) -> wire::Zone {
    wire::Zone {
        id,
        row,
        label,
        layout: layout as i32,
        cards,
        count: count as u32,
        position: None,
    }
}
