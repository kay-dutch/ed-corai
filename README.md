# ED-Corai

**Ship builds for Elite Dangerous worked out by the numbers, not by forum consensus.**

[![Licence][badge-licence]](./LICENSE)
[![Release][badge-release]](../../releases)
[![CI][badge-ci]](../../actions/workflows/ci.yml)
[![MCP][badge-mcp]](https://modelcontextprotocol.io/)
[![Rust][badge-rust]](https://www.rust-lang.org/)
[![Node.js][badge-node]](https://nodejs.org/)
[![Elite Dangerous][badge-ed]](https://www.elitedangerous.com/)

ED Corai is an MCP server: it plugs into Claude, and into any other AI app that supports the
[Model Context Protocol](https://modelcontextprotocol.io/) (OpenAI Codex, Gemini CLI, GitHub
Copilot in VS Code, Cursor and others).

Ask for a build in plain words — *"longest-range Krait Mk II with a fuel scoop, AFMU and SRV bay"*,
*"tankiest Fer-de-Lance that still does 150 sustained DPS"*, *"which medium ship carries the most
cargo and still jumps 20 Ly laden?"* — and ED Corai searches the outfitting space, scoring every
candidate with the full outfitting math: mass, power, jump range, speed, shields and hull with
resistances, sustained DPS. You get the build, the numbers behind it, and links to open it in
Coriolis or EDSY.

## What it does

- Finds the best fit for one objective under hard constraints (minimum jump range, required
  modules, cost cap, power budget, weapon consistency, ...).
- Engineering included by default: grade-5 blueprints at best roll plus the best experimental
  effect for each module. Can be switched off for stock builds.
- Compares hulls: runs the same goal over every ship (or a subset) and ranks them.
- Evaluates and tweaks existing builds: paste a Coriolis link, an EDSY export, or a journal
  Loadout, change a slot, see the effect.
- **Converts builds between formats: paste a Coriolis link, an EDSY SLEF export or a journal
  Loadout and get it back as a Coriolis link, an EDSY import link, SLEF or journal JSON (for
  EDSY, Coriolis, Inara and EDDiscovery).**
- Reports constraints it could not meet instead of quietly ignoring them.

### Tools exposed to the assistant

| Tool | Purpose |
|---|---|
| `list_ships` | Hulls with size, price, base stats and slot layout |
| `ship_details` | Slots of a ship and its stock build |
| `list_module_groups` | Module group codes (`fs` fuel scoop, `sg` shield generator, ...) |
| `module_info` | Module stats and the engineering available for a group |
| `list_metrics` | Everything that can be optimised or constrained |
| `evaluate_build` | Full stats of a build, optionally after slot changes |
| `export_build` | A build as SLEF or journal Loadout JSON, plus Coriolis and EDSY links |
| `optimize_build` | Best outfitting of one ship for an objective and constraints |
| `compare_ships` | Same optimisation across many hulls, ranked |
| `check_for_updates` | Installed version, the game data it was built from, and the latest release |
| `install_update` | Updates the install to a release, after you agree |

Builds can be given in any common form: a Coriolis outfit or import link, an EDSY import link,
SLEF text (in EDSY: Export → SLEF), or a journal `Loadout` event. EDSY's short `#/L=` links cannot
be read; use its SLEF export instead.

### Metrics

Travel: jump ranges (unladen, laden, full tank, total per tank), speed, boost, pitch/yaw/roll,
boost interval. Capacity: cargo, passengers, fuel, fuel scoop rate, refinery bins, collector
limpets, dry and laden mass, cost, power margin.

Defence: raw and effective shield HP against a damage mix, worst-type shield HP, shield regen,
recharge and recovery times, time shields last under a given incoming DPS (with or without cell
banks), raw and effective hull HP, module protection.

Offence: sustained and burst DPS, DPS against a reference target's shields and hull, piercing
efficiency against its hardness, time to kill it, weapon capacitor sustain, ammo endurance, weapon
heat, number of weapon types and mount types (for consistent, aimable loadouts).

Heat: net heat per second in a fight (power plant plus weapons, minus what Thermal Vent beams
remove while on target) and the vent strength at which the ship would hold steady.

Plus `count:<group>` / `class:<group>` for "must have" modules.

## How the numbers are computed

Everything is calculated, nothing is looked up from community tier lists. Where the game's own
mechanics are known they are applied directly; where tools disagree or no measurement exists, the
assumption is a setting you can change.

- **Resistances** follow the game's stacking rules: shield booster resistances get diminishing
  returns above 30 %, the generator's own resistance is exempt, SYS pips add
  `60 % × (pips/4)^0.85` on shields only. Hull reinforcement resistances stack one module at a
  time, weakest first: a module's resistance above 30 % counts double, the combined value gets
  diminishing returns between 30 % and 65 %, and the total caps at 75 %.
- **Hardness vs piercing** reduces hull damage by `piercing / hardness` when a weapon's
  penetration is below the target's hardness; this is why piercing efficiency and time to kill are
  reported against a reference opponent.
- **Jump range** counts fuel once and excludes the reserve tank by default, matching the journal's
  `MaxJumpRange` (Coriolis counts fuel twice; `reserveFuelInJumpMass` restores its behaviour).
- **Shields under fire**: regeneration is assumed to pause while taking hits (`regenUnderFire`
  = 0). The real pause is unmeasured; set 0–1 to explore.
- **Corrosive Shell** raises all hull damage to a target it keeps hitting by 25 % (once, however
  many weapons carry it) and adds 20 piercing to its own weapon. Coriolis models only its ammo
  penalty. Both figures are community numbers and can be changed.
- **Thermal Vent** beams generate no heat while on target and remove heat instead. How much they
  remove is unmeasured; the default assumes as much as the beam would otherwise generate.
- **Ammo** is finite: `ammoEndurance` is the time until the first ammo-limited weapon runs dry,
  so missiles and torpedoes are not mistaken for sustained damage. Plasma Slug counts as unlimited.
- **Outfitting rules** mirror the game: life support, sensors and SCO frame shift drives must
  match their slot size; thrusters must be able to lift the hull; one shield generator, fuel
  scoop, refinery, interdictor, scanner of each kind, and so on; at most four experimental
  (AX/Guardian) weapons; restricted slots (military, cargo, limpets, mining hardpoints) and
  ship-specific modules are honoured.
- **Not modelled yet**: heat capacity and time to overheat (net heat per second is), sensor range, module
  breach chance, NPC behaviour. The default `pve` damage mix (30 % kinetic, 60 % thermal,
  5 % explosive, 5 % absolute) is an assumption, not a measurement.

Every default that rests on an assumption is listed by `list_metrics` so you can rerun with the
alternative when a ranking is close.

## Installation

Requires Node.js 20 or newer and git.

```sh
git clone --recursive https://github.com/kay-dutch/ed-corai.git
cd ed-corai
npm install
npm run build
```

Forgot `--recursive`? Run `git submodule update --init` before building.

`npm test` runs the end-to-end checks.

## Configuration

### Claude Code

```sh
claude mcp add ed-corai -s user -- node /absolute/path/to/ed-corai/dist/server.cjs
```

`-s user` makes it available in every project; without it, it only works in the folder where you
ran the command.

### Claude Desktop

Add to `claude_desktop_config.json` (Settings → Developer → Edit Config):

```json
{
  "mcpServers": {
    "ed-corai": {
      "command": "node",
      "args": ["/absolute/path/to/ed-corai/dist/server.cjs"]
    }
  }
}
```

Restart Claude Desktop afterwards. If it reports that `node` cannot be found (common with nvm or
other version managers), put the full path that `which node` prints in `command`.

### Other MCP apps

ED Corai runs on your computer, so it works in any app that can start a local MCP server. Every
app needs the same two things: the command `node` and the full path to `dist/server.cjs`.

**OpenAI Codex CLI**, in `~/.codex/config.toml`:

```toml
[mcp_servers.ed-corai]
command = "node"
args = ["/absolute/path/to/ed-corai/dist/server.cjs"]
```

**Gemini CLI**, in `~/.gemini/settings.json`:

```json
{
  "mcpServers": {
    "ed-corai": {
      "command": "node",
      "args": ["/absolute/path/to/ed-corai/dist/server.cjs"]
    }
  }
}
```

**GitHub Copilot in VS Code** (agent mode), in `.vscode/mcp.json`, or run *MCP: Add Server* from
the command palette:

```json
{
  "servers": {
    "ed-corai": {
      "type": "stdio",
      "command": "node",
      "args": ["/absolute/path/to/ed-corai/dist/server.cjs"]
    }
  }
}
```

**Cursor, Windsurf, Cline, LM Studio** and most other apps take the same `mcpServers` block as
Claude Desktop above, in their own MCP settings file.

**Your own agent**: with the OpenAI Agents SDK, start it as a stdio server:

```python
from agents.mcp import MCPServerStdio

ed_corai = MCPServerStdio(params={"command": "node", "args": ["/absolute/path/to/ed-corai/dist/server.cjs"]})
```

Things to know outside Claude:

- **Use a large model.** A good answer takes several tool calls and reads large results. Big
  models (GPT-5, Gemini Pro, Claude) handle it; small local models often lose the thread.
- **Some apps drop the server's instructions.** ED Corai sends a short guide on how to use its
  tools (lock the slots you are not changing, keep what the build already does as minimums). If
  results look careless, tell the model: *"Read list_metrics first. Lock every slot I don't
  mention."*
- **Ask for updates yourself.** Not every app shows the model the update notice; ask *"check for
  ED Corai updates"* now and then.
- **ChatGPT (app and website)** only connects to MCP servers hosted on the internet, not to
  programs on your computer, so it cannot use ED Corai as it is.

### Options

Most behaviour is chosen per request, and the assistant picks sensible values from your wording. You can
also ask for them directly:

| Option | Default | Meaning |
|---|---|---|
| `engineering` | `full` | `full` = G5 + experimentals, `none` = stock modules |
| `effort` | `normal` | `quick`, `normal` or `thorough` (more restarts, slower) |
| `power` | `priority` | `priority`: combat modules stay powered with hardpoints out while non-combat ones (scoop, docking computer, SRV bay, ...) are shed by priority group, as in game; `deployed`: everything on with hardpoints out; `retracted`; `ignore` |
| `requireBoost` | `true` | Distributor must be able to boost |
| `lock` | — | Slots to keep as they are, e.g. `["hardpoint", "standard:fsd"]` |
| `filters` | everything allowed | Forbid Powerplay, pre-engineered, community-goal, Guardian or AX modules, or specific groups |
| `context.sysPips` / `wepPips` | 2 / 4 | Pips while fighting: shield resistance, regen under fire, weapon sustain |
| `context.recoverySysPips` | 4 | SYS pips while recovering: shield recharge and rebuild times |
| `context.damagePreset` | `even` | `even` or `pve` (thermal-heavy); explicit `damage` weights override |
| `context.incomingDps` | 200 | Raw incoming DPS for time-under-fire metrics |
| `context.regenUnderFire` | 0 | Share of time shields regenerate while under fire |
| `context.opponent` / `rangeMeters` | stock Anaconda / 1000 | Reference target and range for offence metrics |
| `context.ventHitRate` / `thermalVentFactor` | 1 / 1 | Share of time Thermal Vent beams hit, and heat removed per hit relative to the beam's own heat |
| `context.corrosiveHullMultiplier` / `corrosivePiercingBonus` | 1.25 / 20 | Corrosive Shell hull damage multiplier and piercing bonus |
| `context.reserveFuelInJumpMass` | `false` | Count the reserve tank in jump mass (Coriolis behaviour) |

Environment variables for the server:

| Variable | Meaning |
|---|---|
| `CORAI_DEBUG=1` | Log diagnostic output to stderr |
| `CORAI_UPDATE_CHECK=0` | Do not look for new releases at start-up |
| `CORAI_REPO=owner/name` | Check a fork for releases instead of this repository |

Keep your own builds in `builds/` (git-ignored): SLEF files there can be passed to the tools by
path when running the CLI, e.g. `node dist/cli.cjs evaluate builds/my-corvette.slef`.

## Updating

At start-up ED Corai checks GitHub for a newer release, at most once per launch, and the assistant
tells you when one is out. Say yes and it updates itself: it fetches the release, refreshes the
game data, reinstalls and rebuilds. Then restart it (`/mcp` in Claude Code, or restart your app).
You can also ask "is there an update?" at any time.

The automatic update needs a git clone without local changes. Otherwise, or by hand:

```sh
git pull
git submodule update --init
npm install
npm run build
```

Each release states which Coriolis data it was built from; `check_for_updates` shows it for your
install. New ships and modules reach ED Corai once they are in Coriolis.

## Using it

Talk to the assistant as you would to an experienced builder. It turns the request into an objective and
constraints, runs the tools and answers with the numbers and links.

**Find a build**

> Longest-range Krait Mk II with a fuel scoop, AFMU and SRV bay.

> Tankiest Fer-de-Lance that still does 150 sustained DPS. Fixed weapons only, one weapon type.

> Mining Python with the most cargo that keeps 20 Ly laden range.

**Pick a ship**

> Which medium ship gives the most shield for PvE combat zones? Show the top five.

> I have 400 million credits. Best laden jump range for a trader?

**Improve your own build**

Paste a Coriolis link, an EDSY SLEF export or a journal Loadout.

> Here is my Anaconda Mk II: <link>. Keep the weapons and FSD. Make it run cooler without
> losing shields or DPS.

> This is my Corvette <SLEF>. Get the Anaconda <link> as close to it as possible in protection
> and damage.

**What if**

> What happens if I swap the multi-cannon on hardpoint 6 for a Thermal Vent beam?

> Same build, but I fight with 4 pips in SYS. Does the shield choice change?

**Take it to the game**

> Give me SLEF for EDSY and the Coriolis link.

> Open this Coriolis build in EDSY: <link>

> Here is my SLEF from EDSY. Give me a Coriolis link.

### Tips

- Say what must stay. Slots you name as fixed are left alone. Other modules of your build are kept
  unless changing them improves what you asked for, and the answer lists every change.
- Say what must be there: fighter hangar, docking computer, heat sink, a real fuel tank.
- Say how you fly: pips, typical range, PvE or PvP, what you usually fight.
- Ask for alternatives. "What does 10 % more shield cost?" shows the trade-off better than a
  single answer.
- Ask which assumptions a result depends on, and rerun with the other value when it is close.

## Notes

- Results come with power priorities set (combat modules in group 1, non-combat in group 5) and
  internal modules moved to the smallest slots they fit; neither changes any number.
- Results come from a heuristic search (multi-start local search with engineering, relocation and
  repair moves). It finds very strong builds but does not prove global optimality; `thorough`
  effort explores more.
- Engineering is modelled at best roll. Real rolls vary.
- Elite Dangerous is a trademark of Frontier Developments plc. ED Corai is a fan project and is not
  affiliated with or endorsed by Frontier.

## License

MIT, see [LICENSE](LICENSE). The bundled Coriolis engine is also MIT. Ship and module data belong
to Frontier Developments plc and are not covered by this licence; see
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

[badge-licence]: https://img.shields.io/github/license/kay-dutch/ed-corai?style=for-the-badge
[badge-release]: https://img.shields.io/github/v/release/kay-dutch/ed-corai?style=for-the-badge&label=release&color=F07B05
[badge-ci]: https://img.shields.io/github/actions/workflow/status/kay-dutch/ed-corai/ci.yml?branch=main&style=for-the-badge&label=tests
[badge-node]: https://img.shields.io/badge/Node.js-20%2B-F07B05?style=for-the-badge&logo=nodedotjs&logoColor=white
[badge-mcp]: https://img.shields.io/badge/MCP-server-F07B05?style=for-the-badge&logo=anthropic&logoColor=white
[badge-rust]: https://img.shields.io/badge/Rust-%23000000.svg?style=for-the-badge&logo=rust&logoColor=white
[badge-ed]: https://img.shields.io/badge/ELITE%20DANGEROUS-unofficial%20tool-F07B05?style=for-the-badge&logo=data:image/svg%2Bxml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyMDAwIDIwMDAiPjxnPjxwb2x5Z29uIGZpbGw9IiNGMDdCMDUiIHBvaW50cz0iOTk5Ljc3NCwxMDUzLjY4NSAxMDY1LjUwNiw5OTUuOTU0IDExODEuNzgxLDk5NS45NTQgMTE1MS4wODQsOTY5LjQxNyAxMDczLjI4MSw5NTUuOTQ1IDEwNzEuMzgzLDkyOC4xNDIgMTEyNS40NTEsOTI4LjE0MiAxMDk0LjQ0LDkwMi4yMzggMTA1OC4yNzMsODk2LjA5IDEwNDkuMjMxLDg0MC4zMDMgMTA2NC42MDIsODI3LjA1NyAxMDQzLjUzNSw1NzEuOTk0IDEwMzMuMDAxLDYyOS41NDMgMTAyNy4yNiw3NzQuNTI1IDk5OS42ODQsNzk1LjIzIDk3Mi40NjgsNzc0LjUyNSA5NjYuNzI3LDYyOS41NDMgOTU2LjE5NCw1NzEuOTk0IDkzNS4xMjYsODI3LjA1NyA5NTAuNDUxLDg0MC4zMDMgOTQxLjQxLDg5Ni45OTQgOTA1LjI4OSw5MDIuMjM4IDg3NC4yNzcsOTI4LjE0MiA5MjguMyw5MjguMTQyIDkyNi40MDIsOTU1Ljk0NSA4NDguNTk4LDk2OS40MTcgODE3LjkwMyw5OTUuOTU0IDkzNC4yMjIsOTk1Ljk1NCAiLz48cGF0aCBmaWxsPSIjRjA3QjA1IiBkPSJNMTE4OC4xNTYsODIwLjU0N2gtMC40NTJ2MzIuNzc2YzAsMCw2MS4zOTMsNDguMzI3LDYxLjM5MywxMDYuMzc0bC02Ny4yMjUsNjcuODEyaC02OC45ODdsLTUxLjY3Myw1MC40NTIgdjAuMzYybDAsMGwtMy41MjYsMTcuMTc5SDk0Mi41ODZsLTMuNTI2LTE3LjEzNGwwLDB2LTAuMzYxbC01MS42NzMtNTAuMzE3aC02OS4wMzNsLTY3LjIyNC02Ny44MTIgYzAtNTcuODY2LDYxLjQzOC0xMDYuMTAzLDYxLjQzOC0xMDYuMTAzdi0zMy40MDloLTAuNDUyTDAsODYuMzI1QzAuNDUyLDI2MS40NiwwLjQ1MiwyOTEuNzA0LDc1LjMxNywzNjEuNjg3IGMxLjY3MywxLjQ5Miw2Ni45MDcsNjMuMjkxLDc2LjIyMSw3MC42MTR2MS4yNjZjMCwxNjIuMDI1LDAsMTYyLjAyNSw2My4yOTEsMjE2Ljk5OWMxLjk4OSwxLjc2Myw3My43MzQsNjguOTQxLDczLjczNCw2OC45NDEgcy0wLjI3MSwyMi45MjEsMCwzMC4yNDRjMCwxMTYuNDExLDAsMTE3Ljc2OCw1My4zLDE3NS40MDdjMTkuODkyLDIxLjUyLDEwMi43MTIsOTkuNDU4LDE4My45NTEsMTc0LjU5M0gyNjguMTczbDI4OC41MTgsMjU3LjY4NiBoMzE1LjgyM2wtMjguMjEsNDkuNzI5SDYwOC44NjFsNzguOTc5LDY3LjgxM2gyMDUuMjg5bDEwNi45NjItMjEwLjI2M2wxMDkuMjIzLDIxMC4yNjNoMjAyLjI2bDc4Ljk3OS02Ny44MTNoLTIzMi41MDQgbC0yOC4yMS00OS43MjloMzE2LjgxN2wyODUuNDg4LTI1Ny42ODZoLTI1Ny42ODZjODEuMzc1LTc1LjMxNiwxNjMuOTY5LTE1My4wNzQsMTgzLjkwNi0xNzQuNTQ4IGM1My4zOTEtNTcuNjQsNTMuMjU1LTU4Ljc3MSw1My4yNTUtMTc1LjM2MWMwLTcuMzY5LDAtMzAuMjg5LDAtMzAuMjg5czcxLjc0NS02Ny4xMzQsNzMuNzMzLTY4Ljg5NyBjNjMuMjkyLTU1LjE1Myw2My4yOTItNTUuMTUzLDYzLjI5Mi0yMTYuOTk4di0xLjI2NmM5LjQwMy03LjAwNyw3NC4zNjYtNjkuMDc4LDc2LjA0LTcwLjU3IGM3NC44NjMtNjkuOTM3LDc0Ljg2My0xMDAuMjI2LDc1LjMxNi0yNzUuNDk3TDExODguMTU2LDgyMC41NDd6Ii8%2BPHBhdGggZD0iTTEwMDEuNDQ3LDE5MTMuNjc2TDEwMDEuNDQ3LDE5MTMuNjc2TDEwMDEuNDQ3LDE5MTMuNjc2eiIvPjxwYXRoIGZpbGw9IiNGMDdCMDUiIGQ9Ik0xMDAxLjk0NCwxNDAyLjczNWwtMTAyLjAzNCwyNDMuMjY0bDcwLjE2Myw2OC4xMjljLTkuMzEzLDU4LjU0NC0xOC4wODQsMTE3LjEzNC0xOC41MzUsMTI5LjQ3NiBjLTAuNjM0LDI1LjQ5Nyw0Ny40MjMsNjcuODEyLDQ5LjcyOSw3MC4wNzJjMi4zOTYtMi4wOCw1MC40MDYtNDQuNTc1LDQ5LjcyOS03MC4wNzJjLTAuMzE2LTExLjk4LTguNzI1LTY3LjgxMy0xNy43NjctMTI0LjYzOSBsNzAuMTYzLTcyLjk2NkwxMDAxLjk0NCwxNDAyLjczNXoiLz48L2c%2BPC9zdmc%2B
