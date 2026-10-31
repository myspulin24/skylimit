# Bezpečnost

## Jak nahlásit chybu

**Nezakládejte veřejné issue.** U bezpečnostní chyby by to znamenalo zveřejnit
ji dřív, než bude co nainstalovat.

Použijte [soukromé hlášení zranitelnosti](https://github.com/myspulin24/skylimit/security/advisories/new).
Je zabudované v GitHubu, vidí ho jen správce repozitáře a nikam se kvůli němu
nepíše e-mail.

Pomůže, když přiložíte:

- verzi aplikace (**Nastavení › O aplikaci**) a systém,
- co se má stát a co se stane,
- pokud jde o zpracování souboru (záznam Claude Code, log T3 Code), tak ukázku
  toho souboru. Stačí zkrácená a bez obsahu konverzace.

## Co čekat

Skylimit dělá jeden člověk ve volném čase. Žádné lhůty na odpověď proto
neslibuji; platí to, co se dodržet dá:

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
| **Mezipaměť spotřeby T3 Code** | `crates/usage-core/src/t3_history.rs` čte `~/.t3/userdata/usage-scan-cache*.json` (a nic jiného ze složky T3 Code, kde leží i jeho tokeny) a bere z ní jen čísla zpráv ze smazaných záznamů Claude Code. Kdyby četl jiné soubory, uložil cokoli kromě čísel a názvu projektu nebo započítal zprávu dvakrát, je to nález |
| **Stavový řádek Claude Code** | `src-tauri/src/claude_code.rs` zapisuje po kliknutí do `~/.claude/settings.json` klíč `statusLine` a jako `skylimit statusline` čte vstup od Claude Code; `crates/usage-core/src/claude_code.rs` si z něj nechá jen čísla limitů. Kdyby šlo přes tenhle vstup nebo uschovaný původní stavový řádek spustit cizí příkaz, zapsat mimo složku Skylimitu nebo uložit text relace, je to nález |
| **Účet a limity z `.claude.json`** | `crates/usage-core/src/claude_config.rs` čte globální konfiguraci Claude Code (`~/.claude.json`) a deserializuje z ní jen `oauthAccount` a `cachedUsageUtilization`. Kdyby se z tohoto souboru dostalo cokoli dalšího (třeba klíč nebo historie projektů) do databáze, diagnostiky, snímku pro widget nebo zpětné vazby, je to nález |
| **Spouštění po přihlášení** | `src-tauri/src/autostart.rs` po kliknutí zapisuje jediný záznam Skylimitu: hodnotu `Skylimit` v klíči `Run` uživatele, Launch Agent v `~/Library/LaunchAgents` nebo soubor v `~/.config/autostart`, vždy s cestou k vlastnímu programu bez argumentů. Kdyby šlo do záznamu dostat jiný program nebo argumenty, je to nález |
| **Spouštění Codexu** | `crates/usage-core/src/codex.rs` a `CodexSource::official()` spouštějí oficiální `codex app-server` s pevnými argumenty (na Windows v Job Objectu, na macOS a Linuxu přes přihlašovací shell). Kdyby šlo do příkazu něco propašovat, je to nález |
| **Zpětná vazba** | `src-tauri/src/feedback.rs` posílá text na jedinou zakompilovanou adresu (jen https, nebo http na tenhle počítač). Worker ve [`feedback/`](feedback/) ověřuje tvar zprávy a drží token ke GitHubu |
| **Ověřování aktualizací** | `src-tauri/src/updater.rs` stahuje balíček z GitHub Releases a `tauri-plugin-updater` ho ověří podpisem minisign proti klíči v `src-tauri/tauri.conf.json`. Kdyby šlo ověření obejít, dala by se podstrčit „aktualizace“ |
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
- **Kontrola kurzu ČNB a aktualizací.** Jediná spojení, která Skylimit navazuje
  sám: jednou denně stáhne kurz koruny k euru a dolaru a po spuštění a každých
  6 hodin se podívá na `latest.json` v GitHub Releases. Odchází jen běžný
  dotaz, žádná data. Kontrola aktualizací se vypíná v nastavení nebo
  `SKYLIMIT_AUTO_UPDATE=0`.
- **Zpětná vazba odchází na Cloudflare.** Přesně tam ji posílá tlačítko
  **Odeslat**; popisuje to [README](README.md#soukromí-a-oprávnění).
