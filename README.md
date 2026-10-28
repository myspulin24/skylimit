<div align="center">

<img src="docs/images/mascots.png" alt="Sunnie a Dropie" width="132">

# Skylimit

**Widget na plochu, který hlídá limity Claude a Codexu a počítá, kolik z předplatného vytěžíte.**

[![Vydání](https://img.shields.io/github/v/release/myspulin24/skylimit?label=vyd%C3%A1n%C3%AD)](https://github.com/myspulin24/skylimit/releases/latest)
[![CI](https://github.com/myspulin24/skylimit/actions/workflows/ci.yml/badge.svg)](https://github.com/myspulin24/skylimit/actions/workflows/ci.yml)
[![Licence](https://img.shields.io/badge/licence-MIT-blue)](LICENSE)
[![Platformy](https://img.shields.io/badge/platformy-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey)](https://github.com/myspulin24/skylimit/releases/latest)

[Instalace](#instalace) · [Funkce](#funkce) · [Odkud čísla jsou](#odkud-čísla-jsou) · [Vývoj](#vývoj) · [Soukromí](#soukromí-a-oprávnění)

</div>

---

## Přehled

Skylimit je malý plovoucí widget a desktopová aplikace pro všechny, kdo
programují s **Claude Code** a **Codexem** na předplatné. Ukazuje, kolik zbývá
z pětihodinových a týdenních limitů, a přepočítává spotřebu na ceny API, takže
je hned vidět, kolikrát se předplatné vyplatí. Dvě postavičky, **Sunnie**
(Claude) a **Dropie** (Codex), dávají stav najevo náladou: když se blížíte
k limitu, začnou se potit, po milníku slaví a v noci spí.

<div align="center">
<img src="docs/images/widget.jpg" alt="Widget Skylimit: Sunnie a Dropie, limity Claude a Codexu, skóre měsíce" width="300">
</div>

| | |
| --- | --- |
| **Platformy** | Windows 10/11, macOS 11+, Linux |
| **Data** | Jen lokální: záznamy Claude Code, Codex CLI, logy T3 Code; historie v SQLite |
| **Technologie** | Tauri 2, Rust, React 19, TypeScript, SQLite |
| **Jazyk rozhraní** | čeština |
| **Licence** | MIT |

---

## Instalace

Stáhněte instalátor pro svůj systém ze [stránky vydání](https://github.com/myspulin24/skylimit/releases/latest).

| Systém | Soubor | Poznámka |
| --- | --- | --- |
| Windows | `Skylimit_<verze>_x64-setup.exe` | Instaluje se jen pro aktuálního uživatele, bez práv správce. |
| macOS | `Skylimit_<verze>_universal.dmg` | Univerzální balíček (Apple Silicon i Intel). |
| Linux | `Skylimit_<verze>_amd64.AppImage` | Spustitelné bez instalace (`chmod +x`). |
| Linux | `Skylimit_<verze>_amd64.deb` | Pro Debian, Ubuntu a odvozené. |

Instalátory nejsou podepsané komerčním certifikátem, takže je systém při
prvním spuštění zastaví:

- **Windows:** „Systém Windows ochránil počítač“ → **Další informace** →
  **Přesto spustit**.
- **macOS:** aplikace je podepsaná jen ad hoc. Poprvé ji otevřete pravým
  tlačítkem → **Otevřít**, nebo
  `xattr -dr com.apple.quarantine /Applications/Skylimit.app`.
- **Linux:** ikona v oznamovací oblasti potřebuje AppIndicator
  (`libayatana-appindicator3-1`); v GNOME rozšíření *AppIndicator Support*.

Aplikace se zatím neaktualizuje sama. Novou verzi stáhněte ze stránky vydání,
historie i nastavení zůstanou.

### Co Skylimit potřebuje

| Nástroj | K čemu | Bez něj |
| --- | --- | --- |
| [Claude Code](https://code.claude.com) | Skóre Claude ze záznamů v `~/.claude/projects` | Skóre Claude zůstane prázdné |
| [Codex CLI](https://github.com/openai/codex), přihlášený | Limity a spotřeba Codexu přes `codex app-server` | Řádky Codexu ukážou „Codex CLI nenalezen“ |
| [T3 Code](https://github.com/pingdotgg/t3code) (volitelně) | Limity Claude z jeho logů | Místo limitů Claude stojí „limity Claude jen přes T3 Code“ |

---

## Funkce

### Widget

Malé průhledné okno, vždy navrchu, mimo hlavní panel i Alt+Tab. Dá se
přetáhnout za scénu s postavičkami a pamatuje si, kde stálo.

- **Limity 5h a týden** pro Claude i Codex: procenta, odpočet do resetu
  a barva podle zaplnění (zelená, od 75 % oranžová, od 95 % červená).
- **Skóre měsíce v cenách API** s násobkem paušálu („vytěženo 4,6×“)
  a křivkou po dnech.
- **Obnovit** vedle ozubeného kolečka načte všechny zdroje hned, **bublina**
  otevře okno se zpětnou vazbou.
- **Průhlednost** (0–90 %) a **průchod kliknutí**, kdy widget nereaguje na myš.

### Nálady

<div align="center">
<img src="docs/images/moods.gif" alt="Sunnie a Dropie postupně v náladách klid, soustředění, oslava, pot, panika, dřímání, zmatek a spánek" width="560">
</div>

| Nálada | Kdy |
| --- | --- |
| Klid | Limity v pořádku, poslední práce před 5 až 30 minutami. |
| Soustředění | Nová spotřeba v posledních 5 minutách. |
| Oslava | Čtyři sekundy po milníku: násobek paušálu 1×, 2×, 3×, 5×, 10×, denní rekord, série 7, 14 a 30 dní. |
| Pot | Limit 5h nebo týdne je na 75 % a víc (uklidní se pod 70 %). |
| Panika | Limit je na 95 % a víc (uklidní se pod 90 %). |
| Dřímání | Číslo limitu je starší a čeká se na čerstvé. |
| Zmatek | Limity se nedaří zjistit, nebo se změnil formát záznamů Claude Code. |
| Spánek | 30 minut bez práce. |

Když platí víc nálad najednou, vyhraje ta výš v tabulce. Záložka **Stavy**
v aplikaci ukáže všechny s velkým animovaným náhledem.

### Okno aplikace

<div align="center">
<img src="docs/images/month.jpg" alt="Okno aplikace se skóre měsíce, denním grafem a rozpadem podle zdroje, modelu a projektu (demo data)" width="760">
</div>

- **Měsíc:** skóre a násobek paušálu po poskytovatelích, denní graf, série
  dnů a rozpad podle zdroje (terminál, T3 Code, Claude Desktop, Codex),
  modelu a projektu.
- **Stavy:** všechny nálady, stavy řádků limitů, značky u skóre a barvy ikony.
- **Nastavení:** paušál a měna (EUR, CZK), co nepočítat do skóre,
  průhlednost widgetu, průchod kliknutí, smazání historie a **O aplikaci**
  (verze, licence).
- **Zpětná vazba:** formulář pro chybu, změnu nebo vylepšení s volitelnou
  přílohou. Viz [Soukromí](#soukromí-a-oprávnění).

Snímky v tomto README ukazují smyšlená demo data.

---

## Odkud čísla jsou

| Číslo | Zdroj | Jak často |
| --- | --- | --- |
| Limity Codexu | Oficiální `codex app-server` (JSON-RPC, `account/rateLimits/read`) | Každé 2 minuty a po každé změně |
| Spotřeba Codexu | `account/usage/read`: denní součty tokenů | Každých 5 minut |
| Limity Claude | Události `rate_limit_event`, které Claude Agent SDK předává T3 Code a ten je zapisuje do `~/.t3/userdata/logs/provider` | Při práci v T3 Code, kontrola každých 10 s |
| Skóre Claude | Záznamy Claude Code (`~/.claude/projects/**/*.jsonl`) × ceník API (snímek [LiteLLM](https://github.com/BerriAI/litellm)) | Hned po změně souboru |
| Kurz | Denní kurz [ČNB](https://www.cnb.cz) | Jednou denně |

Skóre Codexu je **odhad**: Codex vrací jen denní součty tokenů bez rozpadu
podle modelu, takže se násobí průměrnou sazbou. Ve widgetu je proto označené
značkou „odhad“ a dá se vypnout v nastavení.

**Proč ne přímo z účtu Claude:** podmínky Anthropicu nedovolují aplikacím
třetích stran používat přihlašovací token Claude. Skylimit proto čte jen to,
co už leží na disku. Práci v terminálu nebo v Claude Desktop do logů T3 Code
nikdo nezapíše; když se Claude po posledním záznamu použije jinde, ukáže
widget číslo s vlnovkou jako „poslední známé“.

---

## Konfigurace

Všechno se nastavuje v aplikaci (**Nastavení**). Data leží mimo instalaci,
takže je odinstalace ani nová verze nesmaže:

| Systém | Složka |
| --- | --- |
| Windows | `%LOCALAPPDATA%\skylimit-data` |
| macOS | `~/Library/Application Support/skylimit-data` |
| Linux | `~/.local/share/skylimit-data` |

Uvnitř je `history.sqlite` (historie skóre a limitů), `settings.json`,
pozice oken a `diag/diag.log`. Diagnostický log obsahuje jen metadata, nikdy
text konverzací ani tajemství. Do verze 0.4.0 se složka jmenovala
`ai-usage-data`; novější verze ji při prvním spuštění sama přejmenuje.

| Proměnná prostředí | Účel |
| --- | --- |
| `SKYLIMIT_FEEDBACK_URL` | Jiná adresa pro zpětnou vazbu, třeba místní `wrangler dev` (jen `https://`, nebo `http://` na `127.0.0.1`/`localhost`) |

---

## Vývoj

### Požadavky

- [Node.js](https://nodejs.org) 22.22 nebo novější
- [Rust](https://rustup.rs) stable
- Na Linuxu [systémové závislosti Tauri](https://tauri.app/start/prerequisites/)
  a `libayatana-appindicator3-dev`

### Spuštění

```bash
npm install
npm run tauri dev
```

| Příkaz | Popis |
| --- | --- |
| `npm run tauri dev` | Aplikace s jádrem v Rustu nad vašimi skutečnými daty |
| `npm run dev` | Jen rozhraní v prohlížeči nad ukázkovými daty (`?window=app`, `?mock=panic`, `?window=gallery`) |
| `npm test` | Celá testovací sada |
| `npm run package` | Sestavení vydání včetně instalátoru pro aktuální systém |

### Testy

```bash
npm test
```

| Část | Nástroj | Rozsah |
| --- | --- | --- |
| Rozhraní | `vitest` | Widget, okno aplikace, nastavení, zpětná vazba, formátování |
| Worker | `vitest` | Kontrola tvaru zprávy, issue, limity, chyby GitHubu |
| Jádro | `cargo test -p usage-core` | Ceny, skóre, nálady, čtení záznamů a logů, end-to-end proti podvrženému `codex app-server` |
| Shell | `cargo test -p skylimit` | Nastavení, ikona, odesílání zpětné vazby proti místnímu HTTP serveru |

### Architektura

```
src/                     Rozhraní (React): widget, okno aplikace, dialogy
crates/usage-core/       Jádro bez Tauri: zdroje dat, ceník, skóre, nálady, SQLite
  src/claude.rs            záznamy Claude Code (JSONL)
  src/codex.rs             codex app-server (JSON-RPC)
  src/t3.rs                limity Claude z logů T3 Code
src-tauri/               Shell: okna, ikona v oznamovací oblasti, příkazy, zpětná vazba
feedback/                Cloudflare Worker, který ze zpětné vazby zakládá issue
docs/snapshot-contract.md  Smlouva mezi jádrem a rozhraním
```

Jedno vlákno jádra vlastní databázi a z každé změny spočítá **snímek**
(`Snapshot`), který rozhraní jen vykresluje. Snímek nikdy neobsahuje tajemství
ani text konverzací. Rozhraní v prohlížeči (`npm run dev`) běží nad ukázkovými
daty se stejným tvarem snímku, takže jde ladit bez Rustu.

---

## Soukromí a oprávnění

Skylimit nemá účty, telemetrii ani analytiku. Sám od sebe navazuje jediné
spojení: jednou denně stáhne kurz ČNB. Codex CLI komunikuje s OpenAI sám,
jako oficiální klient; Skylimit jen čte, co vrátí.

Přihlašovací údaje se nečtou nikdy: ani `~/.claude/.credentials.json`, ani
`~/.codex/auth.json`. Z logů T3 Code, ve kterých jsou celé konverzace, se
berou jen čísla limitů.

| Funkce | Co odchází | Kdy |
| --- | --- | --- |
| Kurz ČNB | Dotaz na veřejnou stránku ČNB | Jednou denně |
| Zpětná vazba | Napsaná zpráva, druh, záložka, verze a systém; volitelně jméno, e-mail a příloha | Po stisknutí **Odeslat** |

Zpětná vazba jde přes malý Worker na Cloudflare ([`feedback/`](feedback/)),
který z ní založí issue v soukromém repozitáři autora. Adresa je v aplikaci
pevná a webview ji změnit nemůže. Aplikace nenese žádný token; ten má jen
Worker. IP adresa se nikam neukládá.

Každé okno smí jen své příkazy (systémem vynucený seznam v
[`src-tauri/capabilities/`](src-tauri/capabilities/)):

| Okno | Smí |
| --- | --- |
| Widget | Číst snímek, otevřít okno aplikace, vyžádat obnovení dat, přetahovat se |
| Aplikace | Navíc měnit nastavení, průhlednost a průchod kliknutí, smazat historii, poslat zpětnou vazbu |

---

## Bezpečnost

Bezpečnostní chyby hlaste prosím
[soukromým hlášením](https://github.com/myspulin24/skylimit/security/advisories/new),
nikoli veřejným issue. Rozsah a postup popisuje [SECURITY.md](SECURITY.md).

---

## Přispívání

Chyby a náměty patří do [issues](https://github.com/myspulin24/skylimit/issues),
nebo rovnou do okna **Zpětná vazba** v aplikaci. Před odesláním pull requestu
spusťte `npm test`; stejnou sadu ověřuje CI.

Historie změn je v [CHANGELOG.md](CHANGELOG.md). Text vydání se z něj
přebírá automaticky.

---

## Licence

MIT © 2026 Michal Jašek. Viz [LICENSE](LICENSE).
