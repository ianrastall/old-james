# Old James

Old James is a card-game project for playing with language-model opponents.

## Planned games

- **Multiplayer** — Bridge, Poker, Hearts, Spades, Pinochle, Euchre, Rummy, and
  Gin Rummy (Gin). Poker and Rummy each need a named variant for their first
  ruleset.
- **Solitaire** — Klondike, Double Klondike, Diplomat, Spider One Suit,
  Spider Two Suits, Grandfather Clock, and FreeCell.

Rulesets describe deck composition, visibility, table spaces, legal actions,
and scoring. Card instances have their own identities so Pinochle, Double
Klondike, and Spider can reuse one face image for multiple cards in play.

## Card artwork

Both asset packs use the released **Design 1** artwork from the sibling
`playing-cards` repository at `d1fbb186ae35e870af513b77953620b26fe8e955`.
Each pack includes 52 suited faces, two jokers, and the Prussian Blue back.
The approved sources were verified against `designs/design1/catalog.json`
and resized to one third of their original width and height with Pillow's
Lanczos resampler. The PNGs retain their transparent corners.

| Pack | Image size |
| --- | --- |
| [Poker](assets/cards/design1/poker/) | 250 × 350 px |
| [Bridge](assets/cards/design1/bridge/) | 225 × 350 px |

Suited faces use `faces/<suit>/<rank>.png`, jokers use `faces/jokers/<color>.png`,
and the back is `backs/prussian-blue.png`, all relative to each pack.
The artwork's [MIT license](assets/cards/design1/LICENSE) accompanies the files.
