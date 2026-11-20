use crate::{
    Choice,
    card::{self, Card, Suit},
    wire::{self, zone::Layout::Pile},
};
use serde::Deserialize;

const STOCK: u32 = 1;
const WASTE: u32 = 2;
const FIRST_CELL: u32 = 10;
const PYRAMID: &[&[u32]] = &[
    &[6],
    &[5, 7],
    &[4, 6, 8],
    &[3, 5, 7, 9],
    &[2, 4, 6, 8, 10],
    &[1, 3, 5, 7, 9, 11],
    &[0, 2, 4, 6, 8, 10, 12],
];
const PEAKS: &[&[u32]] = &[
    &[3, 9, 15],
    &[2, 4, 8, 10, 14, 16],
    &[1, 3, 5, 7, 9, 11, 13, 15, 17],
    &[0, 2, 4, 6, 8, 10, 12, 14, 16, 18],
];

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    mode: Mode,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Pyramid,
    Tripeaks,
}

#[derive(Clone)]
struct Cell {
    card: Option<Card>,
    layer: u32,
    column: u32,
}

#[derive(Clone, Copy)]
pub enum Move {
    Draw,
    Recycle,
    Remove(u32),
    Pair(u32, u32),
    ToWaste(u32),
}

#[derive(Clone)]
pub struct Game {
    mode: Mode,
    cells: Vec<Cell>,
    stock: Vec<Card>,
    waste: Vec<Card>,
    moves: usize,
    redeals_left: u8,
}

impl Game {
    pub fn new(rules: Rules, seed: u64) -> Self {
        let layout = match rules.mode {
            Mode::Pyramid => PYRAMID,
            Mode::Tripeaks => PEAKS,
        };
        let mut stock = card::deck(&Suit::ALL, 1, seed);
        let mut cells = Vec::with_capacity(28);
        for (layer, row) in layout.iter().enumerate() {
            for &column in *row {
                cells.push(Cell {
                    card: stock.pop(),
                    layer: layer as u32,
                    column,
                });
            }
        }
        let waste = if rules.mode == Mode::Tripeaks {
            vec![stock.pop().expect("starter card")]
        } else {
            Vec::new()
        };
        Self {
            mode: rules.mode,
            cells,
            stock,
            waste,
            moves: 0,
            redeals_left: if rules.mode == Mode::Pyramid { 2 } else { 0 },
        }
    }

    fn exposed(&self, cell: &Cell) -> bool {
        !self.cells.iter().any(|lower| {
            lower.layer == cell.layer + 1
                && lower.column.abs_diff(cell.column) == 1
                && lower.card.is_some()
        })
    }

    fn remaining(&self) -> usize {
        self.cells.iter().filter(|cell| cell.card.is_some()).count()
    }

    pub fn choices(&self) -> Vec<Choice<Move>> {
        if self.remaining() == 0 {
            return Vec::new();
        }
        let mut choices = Vec::new();
        if !self.stock.is_empty() {
            choices.push(Choice::new(
                Move::Draw,
                "Draw from stock".into(),
                STOCK,
                0,
                0,
            ));
        } else if self.redeals_left > 0 && !self.waste.is_empty() {
            choices.push(Choice::new(
                Move::Recycle,
                "Turn waste over".into(),
                STOCK,
                0,
                0,
            ));
        }
        let mut available: Vec<_> = self
            .cells
            .iter()
            .enumerate()
            .filter(|(_, cell)| self.exposed(cell))
            .filter_map(|(i, cell)| cell.card.map(|card| (FIRST_CELL + i as u32, card)))
            .collect();
        if self.mode == Mode::Tripeaks {
            let rank = self.waste.last().expect("waste card").rank;
            for (id, card) in available {
                if matches!(card.rank.abs_diff(rank), 1 | 12) {
                    choices.push(Choice::new(
                        Move::ToWaste(id),
                        format!("Play {}", card.name()),
                        id,
                        0,
                        0,
                    ));
                }
            }
        } else {
            if let Some(&card) = self.waste.last() {
                available.push((WASTE, card));
            }
            for &(from, card) in &available {
                if card.rank == 13 {
                    choices.push(Choice::new(
                        Move::Remove(from),
                        format!("Remove {}", card.name()),
                        from,
                        0,
                        0,
                    ));
                } else {
                    for &(to, other) in &available {
                        if from != to && card.rank + other.rank == 13 {
                            choices.push(Choice::new(
                                Move::Pair(from, to),
                                format!("Pair {} with {}", card.name(), other.name()),
                                from,
                                card.id,
                                to,
                            ));
                        }
                    }
                }
            }
        }
        choices
    }

    pub fn hint_priority(action: Move) -> u8 {
        match action {
            Move::Pair(from, to) if from != WASTE && to != WASTE => 0,
            Move::Pair(..) | Move::ToWaste(_) => 1,
            Move::Remove(from) if from != WASTE => 1,
            Move::Remove(_) => 2,
            Move::Draw | Move::Recycle => 3,
        }
    }

    fn take(&mut self, id: u32) -> Card {
        if id == WASTE {
            self.waste.pop().expect("selected waste card")
        } else {
            self.cells[(id - FIRST_CELL) as usize]
                .card
                .take()
                .expect("selected tableau card")
        }
    }

    pub fn apply(&mut self, action: Move) {
        match action {
            Move::Draw => self.waste.push(self.stock.pop().expect("stock card")),
            Move::Recycle => {
                self.stock.extend(self.waste.drain(..).rev());
                self.redeals_left -= 1;
            }
            Move::Remove(id) => {
                self.take(id);
            }
            Move::Pair(from, to) => {
                self.take(from);
                self.take(to);
            }
            Move::ToWaste(id) => {
                let card = self.take(id);
                self.waste.push(card);
            }
        }
        self.moves += 1;
    }

    pub fn table(&self) -> wire::Table {
        let backs = (!self.stock.is_empty())
            .then(card::hidden)
            .into_iter()
            .collect();
        let mut zones = vec![
            card::zone(STOCK, 0, "Stock".into(), Pile, backs, self.stock.len()),
            card::zone(
                WASTE,
                0,
                "Waste".into(),
                Pile,
                self.waste.last().map(|c| c.view()).into_iter().collect(),
                self.waste.len(),
            ),
        ];
        for (i, cell) in self.cells.iter().enumerate() {
            let cards = cell
                .card
                .map(|card| {
                    if self.mode == Mode::Tripeaks && !self.exposed(cell) {
                        card::hidden()
                    } else {
                        card.view()
                    }
                })
                .into_iter()
                .collect();
            let mut zone = card::zone(
                FIRST_CELL + i as u32,
                1,
                format!("Tableau, row {}", cell.layer + 1),
                Pile,
                cards,
                usize::from(cell.card.is_some()),
            );
            zone.position = Some(wire::CardPosition {
                layer: cell.layer,
                column: cell.column,
            });
            zones.push(zone);
        }
        let remaining = self.remaining();
        let status = if remaining == 0 {
            format!("Won in {} moves", self.moves)
        } else {
            let mut status = format!("Tableau cards left: {remaining} · Moves: {}", self.moves);
            if self.mode == Mode::Pyramid {
                status.push_str(&format!(" · Redeals left: {}", self.redeals_left));
            }
            status
        };
        wire::Table {
            zones,
            finished: remaining == 0,
            status,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(rank: u8, suit: Suit) -> Card {
        Card {
            rank,
            suit,
            id: u32::from(rank) + 13 * suit as u32,
        }
    }

    #[test]
    fn pyramid_exposes_a_card_only_after_both_covering_cards_are_removed() {
        let mut game = Game::new(
            Rules {
                mode: Mode::Pyramid,
            },
            0,
        );
        for cell in &mut game.cells {
            cell.card = None;
        }
        game.cells[15].card = Some(card(1, Suit::Clubs));
        game.cells[21].card = Some(card(13, Suit::Clubs));
        game.cells[22].card = Some(card(13, Suit::Hearts));
        game.cells[27].card = Some(card(12, Suit::Clubs));
        let can_pair = |game: &Game| {
            game.choices()
                .iter()
                .any(|choice| matches!(choice.op, Move::Pair(25, 37)))
        };
        assert!(!can_pair(&game));
        game.apply(Move::Remove(31));
        assert!(!can_pair(&game));
        game.apply(Move::Remove(32));
        assert!(can_pair(&game));
        game.apply(Move::Pair(25, 37));
        assert!(game.table().finished);
        assert!(!game.stock.is_empty());
        assert!(game.choices().is_empty());
    }

    #[test]
    fn tripeaks_hides_the_peaks_and_allows_an_ace_on_a_king() {
        let mut game = Game::new(
            Rules {
                mode: Mode::Tripeaks,
            },
            0,
        );
        let table = game.table();
        assert_eq!(
            table
                .zones
                .iter()
                .flat_map(|z| &z.cards)
                .filter(|c| c.hidden)
                .count(),
            19
        );
        assert_eq!(game.stock.len(), 23);
        game.cells[27].card = Some(card(1, Suit::Clubs));
        game.waste = vec![card(13, Suit::Clubs)];
        assert!(
            game.choices()
                .iter()
                .any(|choice| matches!(choice.op, Move::ToWaste(37)))
        );
        game.apply(Move::ToWaste(37));
        assert_eq!(game.waste.last().unwrap().rank, 1);
        assert!(!game.exposed(&game.cells[17]));
        game.cells[26].card = None;
        assert!(game.exposed(&game.cells[17]));
        assert!(!game.table().zones[19].cards[0].hidden);
    }

    #[test]
    fn pyramid_redeals_the_remaining_waste_twice() {
        let mut game = Game::new(
            Rules {
                mode: Mode::Pyramid,
            },
            0,
        );
        game.stock = vec![card(1, Suit::Clubs), card(2, Suit::Clubs)];
        let initial = game.stock.clone();
        for left in (0..=2).rev() {
            assert_eq!(game.stock, initial);
            game.apply(Move::Draw);
            game.apply(Move::Draw);
            let recycle = game
                .choices()
                .into_iter()
                .find(|choice| matches!(choice.op, Move::Recycle));
            if left > 0 {
                game.apply(recycle.unwrap().op);
            } else {
                assert!(recycle.is_none());
            }
        }
    }
}
