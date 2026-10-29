# Změny

Všechny podstatné změny Skylimitu, od nejnovější verze po nejstarší.

Formát vychází z [Keep a Changelog 1.1.0](https://keepachangelog.com/cs/1.1.0/)
a čísla verzí se řídí [Semantic Versioning 2.0.0-rc.2](https://semver.org/spec/v2.0.0-rc.2.html).
Píše se pro toho, kdo aplikaci používá: co se změní na obrazovce, v datech
nebo v instalaci, ne které soubory se upravily.

**Sekce k dané verzi se stane textem vydání na GitHubu i poznámkami v okně
aktualizace.** Vydání bez sekce tady se nesestaví; hlídá to workflow i testy.

## Verzování

Číslo verze má tvar `MAJOR.MINOR.PATCH` (například `0.6.0`), tři nezáporná
celá čísla bez úvodních nul. Vydaná verze se už nikdy nemění: každá oprava
znamená nové číslo.

**Veřejné API Skylimitu** (SemVer, pravidlo 1) tvoří to, na co se spoléhají
uživatelé a jejich data, ne vzhled rozhraní:

- **Data:** složka `skylimit-data`, databáze `history.sqlite` a `settings.json`.
  Nová verze musí přečíst data každé starší verze se stejným `MAJOR`.
- **Aktualizace:** kanál `latest.json` v GitHub Releases a podpisový klíč
  zabudovaný v aplikaci. Nainstalovaná verze musí najít a ověřit každou další.
- **Instalace:** identifikátor `cz.myspulin.skylimit`, název programu
  a podporované systémy (Windows, macOS, Linux).
- **Proměnné prostředí:** `SKYLIMIT_AUTO_UPDATE` a `SKYLIMIT_FEEDBACK_URL`.
- **Zpětná vazba:** tvar zprávy, kterou aplikace posílá Workeru (`POST /v1/feedback`).

Podle toho se zvyšuje:

| Část | Kdy | Příklad |
| --- | --- | --- |
| `PATCH` | Opravy chyb, které nic z veřejného API nemění | `0.6.0` → `0.6.1` |
| `MINOR` | Nové funkce, nastavení s výchozí hodnotou, nové proměnné; zastarání (označení k pozdějšímu odstranění) | `0.6.1` → `0.7.0` |
| `MAJOR` | Cokoli nekompatibilního: data, která nová verze nepřečte, nový podpisový klíč, odebraná funkce, nastavení, proměnná nebo systém | `1.4.2` → `2.0.0` |

Dokud je `MAJOR` nula (`0.y.z`), jde o počáteční vývoj a podle SemVer se
smí změnit cokoli. I tak se tu drží pravidla z tabulky výš a data se při
změně vždy převedou (jako složka `ai-usage-data` ve verzi 0.5.0). Verze
`1.0.0` přijde, až bude veřejné API ustálené.

**Předběžné verze** mají za číslem pomlčku a označení, například
`0.7.0-rc.1`. Mají nižší přednost než stejné číslo bez označení
(`0.7.0-rc.1` < `0.7.0`). Na GitHubu vycházejí jako *pre-release* a updater
v aplikaci je nenabízí. Metadata sestavení (`+…`) se nepoužívají.

Číslo verze je na čtyřech místech, která se mění vždy společně:
`package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`
a `Cargo.lock`. Soulad hlídá `scripts/version.test.mjs`. V aplikaci je verze
v patičce widgetu, v patičce okna aplikace a v **Nastavení › O aplikaci**.

Postup vydání: sekce `[Nevydáno]` se přejmenuje na novou verzi s datem,
zvedne se číslo na všech čtyřech místech, změny se pushnou a po zeleném CI
se vytvoří tag `vX.Y.Z`, ze kterého workflow `release` sestaví a zveřejní
vydání.

## [Nevydáno]

## [0.7.0] — 2026-10-29

### Přidáno

- **Limity Claude z Claude Code v terminálu** (iTerm2, Terminál, Windows
  Terminal, terminál v editoru). V **Nastavení › Claude Code v terminálu**
  stačí kliknout na **Připojit**: Skylimit se stane stavovým řádkem Claude
  Code a po každé odpovědi si vezme procenta 5h a týdenního limitu. Funguje
  vedle T3 Code, platí novější číslo.
- **Vlastní stavový řádek zůstane.** Kdo ho v Claude Code už má, o nic
  nepřijde: Skylimit ho spustí se stejným vstupem a ukáže jeho výstup.
  **Odpojit** vrátí původní nastavení.
- **Příkaz `skylimit statusline`**, který Claude Code spouští. Bez vlastního
  stavového řádku ukáže v terminálu `☀ Claude 5h 24 % · týden 41 %`.

### Změněno

- **Bez zdroje limitů Claude** radí widget „připoj Claude Code v Nastavení“
  místo „limity Claude jen přes T3 Code“. Záložka Stavy popisuje oba zdroje.
- **Limity Claude se kontrolují každých 5 s** místo 10 s.
- **Složka Claude Code se řídí proměnnou `CLAUDE_CONFIG_DIR`**, když je
  nastavená, pro záznamy ke skóre i pro `settings.json`.

### Zabezpečení

- Do `~/.claude/settings.json` Skylimit zapisuje jen po kliknutí na
  **Připojit** nebo **Odpojit**, jen klíč `statusLine` a s kopií předchozí
  verze ve své složce s daty. Ze vstupu stavového řádku si nechá jen čísla
  limitů; pracovní složku, model ani cestu k přepisu neukládá.

## [0.6.0] — 2026-10-28

### Přidáno

- **Automatické aktualizace.** Skylimit se po spuštění a pak každých
  6 hodin podívá na GitHub, jestli nevyšla nová verze. Když ano, ukáže tečku
  na ozubeném kolečku ve widgetu a v okně aplikace pruh s tlačítkem
  **Aktualizovat a restartovat** a s tím, co je nového. Bez kliknutí se nic
  neinstaluje. Na Linuxu se aktualizuje AppImage.
- **Aktualizace v Nastavení › O aplikaci:** ruční kontrola, stav poslední
  kontroly a přepínač „Hledat novou verzi automaticky“. Proměnná
  `SKYLIMIT_AUTO_UPDATE=0` automatickou kontrolu vypne bez ohledu na nastavení.
- **Číslo verze v patičce widgetu**, vedle údaje o poslední aktualizaci dat.
- **Předběžné verze.** Tag s pomlčkou (například `v0.7.0-rc.1`) vyjde na
  GitHubu jako pre-release a updater ho nenabídne.

### Změněno

- **Okno zpětné vazby mluví lidsky.** Místo technických podrobností o tom,
  kam zpráva putuje, je nahoře krátká věta. Verze a systém se ke zprávě
  přidávají dál, jen se o nich v okně nemluví.
- **CHANGELOG podle Keep a Changelog a SemVer 2.0.0-rc.2**, se zásadami
  verzování a celou historií od verze 0.1.0.

### Zabezpečení

- **Podepsané aktualizace.** Balíčky ve vydání jsou podepsané klíčem, jehož
  veřejná část je zabudovaná v aplikaci. Aktualizace s neplatným podpisem se
  odmítne a nenainstaluje.

## [0.5.0] — 2026-10-28

### Přidáno

- **Zpětná vazba z widgetu.** Bublina vedle obnovení a nastavení otevře okno
  aplikace rovnou s formulářem.
- **O aplikaci** v Nastavení: verze, copyright a tlačítko pro zpětnou vazbu.
  Copyright a číslo verze jsou i v patičce okna aplikace.

### Změněno

- **Všude jen Skylimit.** Program se jmenuje `skylimit.exe` (dřív
  `ai-usage.exe`), identifikátor aplikace `cz.myspulin.skylimit` (dřív
  `cz.myspulin.aiusage`) a data leží ve složce `skylimit-data` (dřív
  `ai-usage-data`). Historie a nastavení se při prvním spuštění přesunou
  samy, nic se nekopíruje ani nemaže.
- **Instalátor uklidí po staré verzi:** ukončí běžící `ai-usage.exe`, smaže
  ho a odstraní mezipaměť okna pod starým identifikátorem.
- **Tlačítko Zpětná vazba** v okně aplikace má ikonu a vypadá jako záložky
  vedle něj.

## [0.4.0] — 2026-10-28

První veřejná verze, s repozitářem
[myspulin24/skylimit](https://github.com/myspulin24/skylimit).

### Přidáno

- **Windows, macOS i Linux.** Instalátor pro Windows, univerzální DMG pro Mac
  (Apple Silicon i Intel) a AppImage a balíček `.deb` pro Linux.
- **Codex na macOS a Linuxu** se spouští přes přihlašovací shell, takže se
  najde, i když je nainstalovaný přes Homebrew, npm nebo nvm.
- **Pozice oken na macOS a Linuxu.** Widget i okno aplikace si pamatují, kde
  stály, a okno mimo všechny monitory se vrátí na obrazovku.
- **Zpětná vazba přímo z aplikace.** Tlačítko **Zpětná vazba** v hlavičce
  okna a „Poslat zpětnou vazbu…“ v menu ikony otevřou formulář: chyba,
  změna, nebo vylepšení, zpráva, nepovinné jméno a e-mail a příloha do 2 MB.
  Zpráva skončí v soukromém repozitáři autora.
- **Dokumentace a vydávání:** README, SECURITY.md, licence MIT, CHANGELOG,
  CI na třech systémech a automatické vydání z tagu.

### Změněno

- **Ukázková data** v prohlížečovém režimu a na snímcích používají smyšlené
  projekty.

### Zabezpečení

- **Zpětná vazba bez tajemství v aplikaci.** Zprávu přijímá Worker na
  Cloudflare, který jediný drží přístup ke GitHubu. Adresa je v aplikaci pevná
  (jen `https://`, nebo `http://` na tento počítač), Worker přijme jen přesný
  tvar zprávy, omezí počet odeslání a zmínky `@někdo` z textu zneškodní.
- **Soukromé hlášení zranitelností** v repozitáři. Postup popisuje SECURITY.md.

## [0.3.0] — 2026-10-28

Neveřejná verze, jen místní sestavení.

### Přidáno

- **Tlačítko Obnovit** ve widgetu vedle ozubeného kolečka. Hned přečte limity
  i spotřebu Codexu, záznamy Claude a logy T3 Code, místo aby čekalo na další
  pravidelné čtení.
- **Velký náhled nálad.** V záložce **Stavy** stačí najet myší na obrázek
  nálady (nebo na něj přejít klávesnicí či klepnout) a ukáže se velký
  animovaný náhled obou postaviček.

### Změněno

- **Nové jméno: Skylimit.** Postavičky se jmenují **Sunnie** (Claude)
  a **Dropie** (Codex). Název je v titulcích oken, v hlavičce, v ikoně
  v oznamovací oblasti i v instalátoru.

## [0.2.0] — 2026-10-28

Neveřejná verze, jen místní sestavení.

### Přidáno

- **Limity Claude (5h a týden).** Skylimit je čte z logů T3 Code, kam Claude
  Agent SDK zapisuje události limitů. Když se s Claude pracuje jinde
  (terminál, Claude Desktop), dostane číslo vlnovku jako „poslední známé“; po
  resetu okna bez práce ukáže 0 %. Limity Claude ovlivňují i náladu Sunnie
  a barvu ikony v oznamovací oblasti.
- **Záložka Stavy.** Všech osm nálad s vysvětlením, kdy která nastane, stavy
  řádků limitů, odkud se čísla berou, značky u skóre a barvy ikony. Najetí
  myší na postavičky ve widgetu ukáže, co právě dělají.
- **Průhlednost widgetu** v nastavení (0–90 %). Průhledné je jen pozadí,
  text a postavičky zůstanou čitelné; projeví se hned.

### Opraveno

- **Okno aplikace se přizpůsobí šířce.** Dlouhé názvy projektů se zkrátí
  a částky zůstanou v kartě, rozpady ukazují osm řádků a zbytek po rozbalení,
  karty se řadí do tří, dvou nebo jednoho sloupce a nastavení je na širokém
  okně ve dvou sloupcích.

## [0.1.0] — 2026-10-28

Neveřejná verze, jen místní sestavení. Pracovní název „AI Usage“.

### Přidáno

- **Plovoucí widget**, průhledný, vždy navrchu, mimo hlavní panel i Alt+Tab.
  Dá se přetáhnout za scénu s postavičkami a pamatuje si pozici.
- **Dvě animované postavičky** pro Claude a Codex s osmi náladami (klid,
  soustředění, oslava, pot, panika, dřímání, zmatek, spánek) podle limitů
  a aktivity.
- **Limity Codexu 5h a týden** z oficiálního `codex app-server`: procenta,
  odpočet do resetu, barva podle zaplnění, „poslední známá“ hodnota, když
  Codex neodpovídá.
- **Skóre měsíce v cenách API.** Claude ze záznamů Claude Code
  (`~/.claude/projects`) podle ceníku API včetně cache, dlouhého kontextu
  a vyhledávání na webu; Codex jako odhad z denních součtů tokenů. Částky
  v EUR nebo CZK podle kurzu ČNB.
- **Násobek paušálu** („vytěženo 4,6×“) z ceny předplatného v nastavení
  a **milníky s oslavou**: 1×, 2×, 3×, 5× a 10× paušálu, denní rekord, série
  7, 14 a 30 dní.
- **Okno aplikace:** skóre a násobek po poskytovatelích, denní graf, série
  dnů a rozpad podle zdroje (terminál, T3 Code, Claude Desktop, Codex),
  modelu a projektu.
- **Nastavení:** paušál, měna, započítání odhadu Codexu, zdroje, které se do
  skóre nepočítají, průchod kliknutí a smazání historie.
- **Ikona v oznamovací oblasti** s barvou podle nejvyššího limitu a menu
  (widget, průchod kliknutí, aplikace, pauza, konec).
- **Historie v SQLite** mimo instalaci, takže ji odinstalace nesmaže.
- **Hlídání formátu záznamů Claude Code:** když se formát změní, skóre se
  označí otazníkem místo tichých chybných čísel.
- **Instalátor pro Windows** v češtině, bez práv správce.

### Zabezpečení

- **Žádné přihlašovací údaje.** Skylimit nikdy nečte přihlašovací token Claude
  ani Codexu a nevolá API Anthropicu. Diagnostický log obsahuje jen metadata,
  nikdy text konverzací.

[Nevydáno]: https://github.com/myspulin24/skylimit/compare/v0.7.0...HEAD
[0.7.0]: https://github.com/myspulin24/skylimit/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/myspulin24/skylimit/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/myspulin24/skylimit/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/myspulin24/skylimit/releases/tag/v0.4.0
