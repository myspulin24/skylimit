# Bezpečnost

## Jak nahlásit chybu

**Nezakládejte veřejné issue.** U bezpečnostní chyby by to znamenalo zveřejnit
ji dřív, než bude co nainstalovat.

Použijte [soukromé hlášení zranitelnosti](https://github.com/myspulin24/skylimit/security/advisories/new).
Je zabudované v GitHubu, vidí ho jen správce repozitáře a nikam se kvůli němu
nepíše e-mail.

Pomůže, když přiložíte:

- verzi aplikace a systém (obojí ukazuje okno **Zpětná vazba** pod formulářem),
- co se má stát a co se stane,
- pokud jde o zpracování souboru (záznam Claude Code, log T3 Code), tak ukázku
  toho souboru. Stačí zkrácená a bez obsahu konverzace.

## Co čekat

Skylimit dělá jeden člověk ve volném čase. Žádné lhůty na odpověď proto
nesliby; platí to, co se dodržet dá:

- Opravuje se **jen poslední vydaná verze.** Starší se nezáplatují.
- Oprava vyjde jako běžné vydání a v poznámkách k němu bude napsané, čeho se
  týká.
- Kdo chybu nahlásí, bude uvedený u opravy, pokud si to nebude přát jinak.

## Kde se dá něco najít

Skylimit běží celý na vašem počítači a čte soubory jiných nástrojů. Zajímavá
místa jsou tato:

| Kde | Proč to stojí za pozornost |
| --- | --- |
| **Čtení logů T3 Code** | `crates/usage-core/src/t3.rs` čte soubory, ve kterých jsou celé konverzace. Smí si z nich vzít jen čísla limitů z `rate_limit_event`; cokoli jiného, co by z nich uniklo do databáze, diagnostiky nebo rozhraní, je nález |
| **Záznamy Claude Code** | `crates/usage-core/src/claude.rs` čte `~/.claude/projects/**/*.jsonl` po řádcích a ukládá jen počty tokenů, model a název složky projektu |
| **Spouštění Codexu** | `crates/usage-core/src/codex.rs` a `CodexSource::official()` spouštějí oficiální `codex app-server` s pevnými argumenty (na Windows v Job Objectu, na macOS a Linuxu přes přihlašovací shell). Kdyby šlo do příkazu něco propašovat, je to nález |
| **Zpětná vazba** | `src-tauri/src/feedback.rs` posílá text na jedinou zakompilovanou adresu (jen https, nebo http na tenhle počítač). Worker ve [`feedback/`](feedback/) ověřuje tvar zprávy a drží token ke GitHubu |
| **Oprávnění oken** | `src-tauri/capabilities/` určuje, které příkazy smí widget a které okno aplikace. Widget nesmí měnit nastavení ani mazat historii |

## Co chybou není

Tyto věci jsou známé, zamýšlené a popsané v [README](README.md), hlásit je
nemusíte:

- **Varování při instalaci.** Instalátory nejsou podepsané komerčním
  certifikátem: Windows ukáže „Systém Windows ochránil počítač“, macOS
  „nelze ověřit vývojáře“. Viz [Instalace](README.md#instalace).
- **Skylimit čte soubory Claude Code, Codexu a T3 Code.** To je jeho práce.
  Nikdy ale nečte přihlašovací údaje (`~/.claude/.credentials.json`,
  `~/.codex/auth.json`) a nevolá API Anthropicu.
- **Kontrola kurzu ČNB.** Jediné spojení, které Skylimit navazuje sám: jednou
  denně stáhne kurz koruny k euru a dolaru. Odchází jen běžný dotaz na veřejnou
  stránku ČNB, žádná data.
- **Zpětná vazba odchází na Cloudflare.** Přesně tam ji posílá tlačítko
  **Odeslat** a okno to říká dřív, než se odešle.
