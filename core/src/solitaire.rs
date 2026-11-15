use crate::{
    Choice,
    card::{self, Card, Suit},
    wire,
};
use serde::Deserialize;

const STOCK: u32 = 1;
const WASTE: u32 = 2;
const FOUNDATION: u32 = 10;
const CELL: u32 = 30;
const COLUMN: u32 = 40;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    suits: Vec<Suit>,
    copies: usize,
    deal: Vec<usize>,
    face_up: FaceUp,
    build: Build,
    empty: Empty,
    groups: Groups,
    stock: Stock,
    foundations: Foundations,
    #[serde(default)]
    cells: usize,
    #[serde(default)]
    return_from_foundation: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum FaceUp {
    All,
    Top,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Build {
    Rank,
    Alternate,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Empty {
    Any,
    King,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Groups {
    Single,
    InOrder,
    SameSuit,
    FreeCell,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
enum Stock {
    None,
    Waste { draw: usize, recycle: bool },
    Row,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
enum Foundations {
    Ace { piles: usize },
    Runs { piles: usize },
    Clock { seeds: Vec<ClockSeed> },
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClockSeed {
    suit: Suit,
    rank: u8,
    stop: u8,
}

#[derive(Clone)]
struct Pile {
    cards: Vec<Card>,
    first_up: usize,
}

impl Pile {
    fn reveal_top(&mut self) {
        if self.first_up == self.cards.len() {
            self.first_up = self.cards.len().saturating_sub(1);
        }
    }
}

#[derive(Clone)]
pub struct Game {
    rules: Rules,
    stock: Vec<Card>,
    waste: Vec<Card>,
    columns: Vec<Pile>,
    cells: Vec<Option<Card>>,
    foundations: Vec<Vec<Card>>,
    total: usize,
    moves: usize,
}

#[derive(Clone, Copy)]
pub enum Move {
    Draw,
    Transfer { from: u32, start: usize, to: u32 },
}

impl Rules {
    pub fn validate(&self) -> Result<(), String> {
        let invalid = || Err("Invalid solitaire deck, layout, or foundation configuration.".into());
        if self.suits.is_empty()
            || self.suits.len() > 4
            || self.copies == 0
            || self.copies > 8
            || self.deal.is_empty()
            || self.deal.len() > 12
            || self.cells > 8
            || self.deal.iter().any(|&n| n > 104)
            || self
                .suits
                .iter()
                .enumerate()
                .any(|(i, s)| self.suits[..i].contains(s))
        {
            return invalid();
        }
        let total = self.suits.len() * self.copies * 13;
        let dealt = self.deal.iter().sum::<usize>();
        let starting = match &self.foundations {
            Foundations::Ace { piles } | Foundations::Runs { piles } => {
                if *piles != total / 13 || *piles > 12 {
                    return invalid();
                }
                0
            }
            Foundations::Clock { seeds } => {
                if self.copies != 1
                    || seeds.len() != 12
                    || seeds.iter().enumerate().any(|(i, seed)| {
                        !(1..=13).contains(&seed.rank)
                            || !(1..=13).contains(&seed.stop)
                            || !self.suits.contains(&seed.suit)
                            || seeds[..i]
                                .iter()
                                .any(|s| s.suit == seed.suit && s.rank == seed.rank)
                    })
                {
                    return invalid();
                }
                let mut covered = Vec::new();
                for seed in seeds {
                    let mut rank = seed.rank;
                    loop {
                        if covered.contains(&(seed.suit, rank)) {
                            return invalid();
                        }
                        covered.push((seed.suit, rank));
                        if rank == seed.stop {
                            break;
                        }
                        rank = rank % 13 + 1;
                    }
                }
                if covered.len() != total {
                    return invalid();
                }
                seeds.len()
            }
        };
        if dealt + starting > total {
            return invalid();
        }
        let remaining = total - dealt - starting;
        if (self.stock == Stock::None && remaining != 0)
            || matches!(self.stock, Stock::Waste { draw, .. } if !(1..=3).contains(&draw))
            || (self.stock == Stock::Row && remaining % self.deal.len() != 0)
            || (self.return_from_foundation && !matches!(self.foundations, Foundations::Ace { .. }))
        {
            return invalid();
        }
        Ok(())
    }
}

impl Game {
    pub fn new(rules: Rules, seed: u64) -> Self {
        let mut stock = card::deck(&rules.suits, rules.copies, seed);
        let total = stock.len();
        let foundations = match &rules.foundations {
            Foundations::Ace { piles } | Foundations::Runs { piles } => vec![Vec::new(); *piles],
            Foundations::Clock { seeds } => seeds
                .iter()
                .map(|seed| {
                    let index = stock
                        .iter()
                        .position(|c| c.rank == seed.rank && c.suit == seed.suit)
                        .expect("validated clock seed");
                    vec![stock.remove(index)]
                })
                .collect(),
        };
        let mut columns: Vec<Pile> = rules
            .deal
            .iter()
            .map(|_| Pile {
                cards: Vec::new(),
                first_up: 0,
            })
            .collect();
        for layer in 0..rules.deal.iter().copied().max().unwrap_or_default() {
            for (column, &size) in columns.iter_mut().zip(&rules.deal) {
                if layer < size {
                    column.cards.push(stock.pop().expect("validated deal"));
                }
            }
        }
        if rules.face_up == FaceUp::Top {
            for column in &mut columns {
                column.first_up = column.cards.len().saturating_sub(1);
            }
        }
        Self {
            cells: vec![None; rules.cells],
            rules,
            stock,
            waste: Vec::new(),
            columns,
            foundations,
            total,
            moves: 0,
        }
    }

    pub fn won(&self) -> bool {
        self.foundations.iter().map(Vec::len).sum::<usize>() == self.total
    }

    pub fn choices(&self) -> Vec<Choice<Move>> {
        let mut actions = Vec::new();
        if self.won() {
            return actions;
        }
        let draw_label = match self.rules.stock {
            Stock::None => None,
            Stock::Waste { .. } if !self.stock.is_empty() => Some("Draw from stock"),
            Stock::Waste { recycle: true, .. } if !self.waste.is_empty() => Some("Turn waste over"),
            Stock::Row
                if !self.stock.is_empty() && self.columns.iter().all(|p| !p.cards.is_empty()) =>
            {
                Some("Deal a row")
            }
            _ => None,
        };
        if let Some(label) = draw_label {
            actions.push(Choice::new(Move::Draw, label.into(), STOCK, 0, 0));
        }
        for (i, column) in self.columns.iter().enumerate() {
            for start in (column.first_up..column.cards.len()).rev() {
                let cards = &column.cards[start..];
                if cards.len() > 1 && !self.group_link(cards[0], cards[1]) {
                    break;
                }
                self.transfers(&mut actions, COLUMN + i as u32, start, cards);
            }
        }
        if let Some(c) = self.waste.last() {
            self.transfers(&mut actions, WASTE, self.waste.len() - 1, &[*c]);
        }
        for (i, cell) in self.cells.iter().enumerate() {
            if let Some(c) = cell {
                self.transfers(&mut actions, CELL + i as u32, 0, &[*c]);
            }
        }
        if self.rules.return_from_foundation {
            for (i, pile) in self.foundations.iter().enumerate() {
                if let Some(c) = pile.last() {
                    self.transfers(&mut actions, FOUNDATION + i as u32, pile.len() - 1, &[*c]);
                }
            }
        }
        actions
    }

    fn group_link(&self, upper: Card, lower: Card) -> bool {
        match self.rules.groups {
            Groups::Single => false,
            Groups::SameSuit => upper.rank == lower.rank + 1 && upper.suit == lower.suit,
            Groups::InOrder | Groups::FreeCell => self.builds_on(lower, upper),
        }
    }

    fn builds_on(&self, card: Card, top: Card) -> bool {
        card.rank + 1 == top.rank
            && (self.rules.build == Build::Rank || card.suit.red() != top.suit.red())
    }

    fn transfers(&self, actions: &mut Vec<Choice<Move>>, from: u32, start: usize, cards: &[Card]) {
        let card = cards[0];
        let mut offer = |to, label: String| {
            if to != from {
                actions.push(Choice::new(
                    Move::Transfer { from, start, to },
                    label,
                    from,
                    card.id,
                    to,
                ));
            }
        };
        if cards.len() == 1 && (from < FOUNDATION || from >= CELL) {
            for (i, pile) in self.foundations.iter().enumerate() {
                let accepts = match &self.rules.foundations {
                    Foundations::Ace { .. } => pile.last().map_or(card.rank == 1, |last| {
                        last.suit == card.suit && last.rank + 1 == card.rank
                    }),
                    Foundations::Runs { .. } => false,
                    Foundations::Clock { seeds } => pile.last().is_some_and(|last| {
                        last.rank != seeds[i].stop
                            && last.suit == card.suit
                            && last.rank % 13 + 1 == card.rank
                    }),
                };
                if accepts {
                    offer(
                        FOUNDATION + i as u32,
                        format!("{} to {}", card.name(), self.foundation_label(i)),
                    );
                }
            }
        }
        if cards.len() == 1 && !(CELL..COLUMN).contains(&from) {
            for (i, cell) in self.cells.iter().enumerate() {
                if cell.is_none() {
                    offer(
                        CELL + i as u32,
                        format!("{} to free cell {}", card.name(), i + 1),
                    );
                }
            }
        }
        for (i, pile) in self.columns.iter().enumerate() {
            if !pile
                .cards
                .last()
                .map_or(self.rules.empty == Empty::Any || card.rank == 13, |top| {
                    self.builds_on(card, *top)
                })
            {
                continue;
            }
            if self.rules.groups == Groups::FreeCell && cards.len() > 1 {
                let empty_columns = self.columns.iter().filter(|p| p.cards.is_empty()).count()
                    - usize::from(pile.cards.is_empty());
                let empty_cells = self.cells.iter().filter(|c| c.is_none()).count();
                if cards.len() > (empty_cells + 1) * (1 << empty_columns) {
                    continue;
                }
            }
            offer(
                COLUMN + i as u32,
                format!("{} to column {}", card.name(), i + 1),
            );
        }
    }

    pub fn apply(&mut self, action: Move) {
        match action {
            Move::Draw => match self.rules.stock {
                Stock::Waste { draw, .. } if !self.stock.is_empty() => {
                    self.waste
                        .extend((0..draw).filter_map(|_| self.stock.pop()));
                }
                Stock::Waste { .. } => self.stock.extend(self.waste.drain(..).rev()),
                Stock::Row => {
                    for pile in &mut self.columns {
                        pile.cards
                            .push(self.stock.pop().expect("a complete stock row"));
                    }
                }
                Stock::None => unreachable!("cannot draw without a stock"),
            },
            Move::Transfer { from, start, to } => {
                let cards = if from >= COLUMN {
                    let pile = &mut self.columns[(from - COLUMN) as usize];
                    let cards = pile.cards.split_off(start);
                    pile.reveal_top();
                    cards
                } else if from >= CELL {
                    vec![
                        self.cells[(from - CELL) as usize]
                            .take()
                            .expect("selected cell"),
                    ]
                } else if from >= FOUNDATION {
                    self.foundations[(from - FOUNDATION) as usize].split_off(start)
                } else {
                    self.waste.split_off(start)
                };
                if to >= COLUMN {
                    self.columns[(to - COLUMN) as usize].cards.extend(cards);
                } else if to >= CELL {
                    self.cells[(to - CELL) as usize] = cards.first().copied();
                } else {
                    self.foundations[(to - FOUNDATION) as usize].extend(cards);
                }
            }
        }
        if matches!(self.rules.foundations, Foundations::Runs { .. }) {
            for pile in &mut self.columns {
                loop {
                    let Some(start) = pile.cards.len().checked_sub(13) else {
                        break;
                    };
                    let run = &pile.cards[start..];
                    if start < pile.first_up
                        || !run
                            .iter()
                            .enumerate()
                            .all(|(i, c)| c.rank == 13 - i as u8 && c.suit == run[0].suit)
                    {
                        break;
                    }
                    let target = self
                        .foundations
                        .iter_mut()
                        .find(|run| run.is_empty())
                        .expect("space for completed suit");
                    *target = pile.cards.split_off(start);
                    pile.reveal_top();
                }
            }
        }
        self.moves += 1;
    }

    fn foundation_label(&self, i: usize) -> String {
        match &self.rules.foundations {
            Foundations::Clock { .. } => format!("{} o’clock", i + 1),
            Foundations::Runs { .. } => format!("Run {}", i + 1),
            Foundations::Ace { .. } => format!("Foundation {}", i + 1),
        }
    }

    pub fn table(&self) -> wire::Table {
        use wire::zone::Layout::{Column, Pile};
        let mut zones = Vec::new();
        if self.rules.stock != Stock::None {
            zones.push(card::zone(
                STOCK,
                0,
                "Stock".into(),
                Pile,
                if self.stock.is_empty() {
                    vec![]
                } else {
                    vec![card::hidden()]
                },
                self.stock.len(),
            ));
        }
        if matches!(self.rules.stock, Stock::Waste { .. }) {
            zones.push(card::zone(
                WASTE,
                0,
                "Waste".into(),
                Pile,
                self.waste.last().map(|c| c.view()).into_iter().collect(),
                self.waste.len(),
            ));
        }
        for (i, cell) in self.cells.iter().enumerate() {
            zones.push(card::zone(
                CELL + i as u32,
                0,
                format!("Free cell {}", i + 1),
                Pile,
                cell.map(Card::view).into_iter().collect(),
                usize::from(cell.is_some()),
            ));
        }
        for (i, pile) in self.foundations.iter().enumerate() {
            let row = if matches!(self.rules.foundations, Foundations::Clock { .. }) && i >= 6 {
                1
            } else {
                0
            };
            zones.push(card::zone(
                FOUNDATION + i as u32,
                row,
                self.foundation_label(i),
                Pile,
                pile.last().map(|c| c.view()).into_iter().collect(),
                pile.len(),
            ));
        }
        for (i, column) in self.columns.iter().enumerate() {
            let cards = column
                .cards
                .iter()
                .enumerate()
                .map(|(j, c)| {
                    if j < column.first_up {
                        card::hidden()
                    } else {
                        c.view()
                    }
                })
                .collect();
            zones.push(card::zone(
                COLUMN + i as u32,
                2,
                format!("Column {}", i + 1),
                Column,
                cards,
                column.cards.len(),
            ));
        }
        let won = self.won();
        wire::Table {
            zones,
            status: if won {
                format!("Won in {} moves", self.moves)
            } else {
                format!("Moves: {}", self.moves)
            },
            finished: won,
            ..Default::default()
        }
    }
}
