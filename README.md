# Old James

Old James is a Windows card table for solitaire, Blackjack, and games against a
local language model. Open **Old James** from Start and choose a game. Click
an eligible card, then a marked destination or an action. In trick-taking
games, click an eligible card to play it. **Rules** shows the selected variant.

[Download the Windows installer](https://github.com/ianrastall/old-james/releases)
or [report a problem](https://github.com/ianrastall/old-james/issues).

## Games

- **Poker** — Heads-up fixed-limit Texas Hold’em, Omaha Hi, Five-Card Draw,
  and Seven-Card Stud. All use play chips.
- **Blackjack** — Single-deck play against a built-in dealer using play chips.
- **Other multiplayer games** — Contract Bridge with raw duplicate scoring,
  Hearts, Spades, single-deck Pinochle, Euchre, Basic Rummy, Gin Rummy, and Cribbage.
- **Solitaire**
  - Klondike and Double Klondike (Draw One and Draw Three), and Westcliff.
  - Yukon, Russian Solitaire, Alaska, and Australian Patience.
  - Diplomat, Rows of Four, Lady Palk, Congress, Parliament, and Forty Thieves.
  - Pyramid and TriPeaks, both won by clearing the tableau.
  - Spider One Suit, Spider Two Suits, Grandfather’s Clock, and FreeCell.

The original Klondike and Double Klondike entries draw one card at a time.
Both Draw Three entries allow unlimited redeals. Rules for the individual
variants are available from **Rules** in the app.

In solitaire, select **Hint** or press **Ctrl+H** to see a suggested legal
move. The hint uses the visible table and leaves the move for you to play.

Open **Game → Deal number…** in solitaire to restart the current deal, play a
specific deal number, or copy the displayed number. Share the number along with
the game variant. For custom games, use the same ruleset file. Playing a
numbered deal replaces that game's saved table.

Variant references include [Yukon](https://pysolfc.sourceforge.io/doc/rules/yukon.html),
[Australian Patience](https://www.goodsol.com/pgshelp/australian_patience.htm),
[Rows of Four](https://www.goodsol.com/pgshelp/rows_of_four.htm),
[Parliament](https://pysolfc.sourceforge.io/doc/rules/parliament.html),
[Westcliff](https://www.bvssolitaire.com/rules/westcliff.htm),
[Pyramid](https://www.goodsol.com/pgshelp/pyramid_clear.htm),
[TriPeaks](https://pysolfc.sourceforge.io/doc/rules/threepeaks.html), and
[Cribbage](https://www.cribbage.org/NewSite/rules/).

Euchre has built-in opponents. Each seat bids, discards and plays from its own
hand, the upcard and the cards already played, and moves without a model. On
your turn, select **Hint** or press **Ctrl+H** to see what the built-in player
would do. The suggested card or action is marked and left for you to play.

The **Settings** dialog accepts a local GGUF model for the other multiplayer
games. The installer bundles llama.cpp. Each opponent receives its own visible
position, the selected rules, public play, and the legal actions for its
current turn. If inference pauses, use **Resume opponent** to retry. Euchre,
Blackjack and solitaire run without a model.

Each game saves its table locally after every move, including solitaire undo
history. Select a game to return to its table. Reopening Old James returns to
the last game you opened. **New game** replaces the selected game's saved table.

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
MSVC build tools, and Inno Setup 6. Clone [Jupiter.UI](https://github.com/ianrastall/jupiter-ui)
into a `jupiter-ui` folder alongside `old-james`.
Run `pwsh -File build.ps1` to build `dist/Old-James-Setup.exe`. The installer
includes the app runtimes and can seed the local GGUF path via
`/MODELFILE="D:\path\to\model.gguf"`. Settings and custom rulesets are stored
under `%LOCALAPPDATA%\Old James Cards`, along with saved games.

Run `cargo test --locked --manifest-path core/Cargo.toml` for the rules engine
checks. The WinUI build runs the Jupiter style check automatically.

## Card artwork

Old James uses the approved **Standard** English-pattern SVG deck from the
sibling `playing-cards` repository at
`3db4b67955d66fc0800abbf72ea9c57a404b4284`. Each pack includes 52 suited faces
and the blue lattice back.

| Pack | SVG viewBox size |
| --- | --- |
| [Poker](assets/cards/standard/poker/) | 240 × 336 |
| [Bridge](assets/cards/standard/bridge/) | 216 × 336 |

Suited faces use `faces/<suit>/<rank>.svg` and the back is `backs/blue.svg`.
The bundled copies expand SVG symbols, references, and lattice tiles into
vector shapes for WinUI. The faces adapt Adrian Kennard's SVG playing cards.
The artwork's [CC0 license](assets/cards/standard/LICENSE) accompanies the files.
The app icon uses the Design 1 ace and back under their
[MIT license](assets/OldJames-icon.LICENSE).
