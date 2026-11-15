# Old James

Old James is a Windows card table for playing solitaire or playing against a
local language model. Open **Old James** from Start and choose a game. Click
an eligible card, then a marked destination or an action. In trick-taking
games, click an eligible card to play it. **Rules** shows the selected variant.

## Games

- **Multiplayer** — Contract Bridge with raw duplicate scoring, heads-up
  fixed-limit Texas Hold'em with play chips, Hearts, Spades, single-deck
  Pinochle, Euchre, two-player Basic Rummy, and Gin Rummy.
- **Solitaire** — Klondike, Double Klondike, Diplomat, Spider One Suit,
  Spider Two Suits, Grandfather's Clock, and FreeCell. Both Klondike games
  use draw one and allow unlimited redeals.

The **Settings** dialog accepts a local GGUF model for multiplayer. The
installer bundles llama.cpp. Each opponent receives its own visible position,
the selected rules, public play, and the legal actions for its current turn.
If inference pauses, use **Resume opponent** to retry. Solitaire does not
load a model and supports undo. Active games are held in memory.

### Add a game

Open **Game → Open ruleset folder**, place a version 1 JSON file there, then
select **Game → Reload games**. Use the [bundled rulesets](rulesets/) as
examples. The engine validates supported rules and fields when it loads each
file. The solitaire family lets rulesets configure deck composition, the
deal, column building, movable groups, stock, cells, and foundations.
Multiplayer files select a supported rules family and its parameters. A
mechanic outside those families needs a new engine primitive.

## Build and install

Build on Windows 11 24H2 or later with .NET SDK 10.0.113, Rust with the x64
MSVC build tools, Inno Setup 6, and the sibling `jupiter-ui` repository.
Run `pwsh -File build.ps1` to build `dist/Old-James-Setup.exe`. The installer
includes the app runtimes and can seed the local GGUF path via
`/MODELFILE="D:\path\to\model.gguf"`. Settings and custom rulesets are stored
under `%LOCALAPPDATA%\Old James Cards`.

Run `cargo test --locked --manifest-path core/Cargo.toml` for the rules engine
checks. The WinUI build runs the Jupiter style check automatically.

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
