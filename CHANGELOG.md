# Changelog

## v0.3.0 — desktop window and your ships

* **Desktop window.** `ed-corai` shows what ED Corai is doing: a Coriolis station that turns while
  it works, the request and the AI app that sent it, recent results with Coriolis links, the
  connected AI apps and the ship you fly now. Closing it never affects your AI app.
* **Your ships from the game.** New tools `current_ship` and `recent_ships` read the game journal,
  so you can ask about "my ship" without pasting a build.
* **Defaults for your AI app.** Set your usual pips, damage mix and engineering in the window; ED
  Corai uses them when a request does not say otherwise and tells the assistant.
* **Updates for both programs**, from the assistant, the window or `ed-corai-cli update`.
* `ed-corai-cli tool` runs every tool, including the journal and update tools.
* Windows programs carry an icon and version details.
* **Fix: builds from the game journal.** Loadouts written by the game (journal, `current_ship`)
  were read with EDSY's units: resistances, shield and hull boosts and thruster and shield
  strength came out 100 times too large, giving impossible speeds and shield figures. ED Corai now
  recognises which program wrote a Loadout. Stock fitted modules (the fighter bay of an Anaconda
  Mk II or Caspian) are no longer lost, and engineered cargo racks hold the game's whole tonnes.
  Imported ships now match the game's own jump range, mass and cargo.

## v0.2.0 — native programs

* **Download and run.** ED Corai now comes as a single program for Windows and macOS (Apple
  silicon and Intel): no Node.js, git or build step. Unzip it and point your AI app at
  `ed-corai-cli` with the argument `mcp`; the README has the setup for each app.
* **Faster searches.** Optimisation runs 8 to 25 times faster and finds equal or better builds;
  ship comparisons search all hulls in parallel.
* **Self-update.** When a new release is out, the assistant offers it; after you agree, the
  program downloads it, checks it against the published checksum and replaces itself. Or run
  `ed-corai-cli update`.
* **Command line.** `ed-corai-cli evaluate`, `optimize`, `tool`, `version`, `check-update` and
  `update` work without an AI app.
* **Fixes.** Share links carry current power priorities after modules are moved; the outfitting
  rules now match Coriolis for maximum module mass, Mk II hangars and the Anaconda Mk II volley
  repeater slots; journal imports name slots correctly for every ship; build exports keep exact
  engineering values.
* The Node.js version stays in the repository and keeps updating itself with git.

## v0.1.0 — first release

* **Builds by the numbers.** Ask Claude for a build in plain words; ED Corai turns it into an
  objective and hard constraints and searches the outfitting space with the Coriolis engine,
  grade-5 engineering and experimental effects included.
* **Your own builds.** Paste a Coriolis link, an EDSY SLEF export or a journal Loadout to evaluate
  it, try changes or improve it. Slots you do not mention keep their modules when nothing is
  gained by changing them.
* **Ship comparison** across all hulls or a size class, ranked by the same goal.
* **Combat metrics** beyond the shipyard tools: effective shield and hull HP against a damage mix,
  time under fire, piercing against armour hardness, time to kill, weapon sustain, ammo endurance,
  net heat with Thermal Vent, Corrosive Shell.
* **Game rules enforced:** slot sizes, unique modules, restricted slots, Guardian modules without
  engineering, power priorities.
* **Export** to SLEF and journal JSON, with Coriolis and EDSY links.
* **Updates from inside Claude.** A new release is offered on start-up and installs after you
  agree.
* Includes the Anaconda Mk II.
