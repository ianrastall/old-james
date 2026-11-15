# Old James

Old James is a card-game project for playing with language-model opponents.

## Card artwork

The bundled deck uses the released **Design 1** Poker artwork from the sibling
`playing-cards` repository at `d1fbb186ae35e870af513b77953620b26fe8e955`.
The [asset pack](assets/cards/design1/) includes 52 suited faces, two jokers,
and the Prussian Blue back. Each approved 750 × 1050 source was verified
against `designs/design1/catalog.json` and resized to 250 × 350 pixels with
Pillow's Lanczos resampler. The PNGs retain their transparent corners.

Suited faces use `faces/<suit>/<rank>.png`, jokers use `faces/jokers/<color>.png`,
and the back is `backs/prussian-blue.png`, all relative to the asset pack.
The artwork's [MIT license](assets/cards/design1/LICENSE) accompanies the files.
