# Změny

Co v které verzi přibylo nebo se opravilo. Nejnovější verze je nahoře.

**Sekce k dané verzi se stane textem vydání na GitHubu.** Vydání bez záznamu
tady se nesestaví; hlídá to workflow, aby nevyšla verze, u které nikdo neví,
co přinesla. Píše se pro toho, kdo aplikaci používá: co se změní na obrazovce,
ne které soubory se upravily.

## 0.4.0 — 2026-10-28

### Přidáno

- **Windows, macOS i Linux.** Skylimit vychází pro všechny tři systémy:
  instalátor pro Windows, univerzální DMG pro Mac (Apple Silicon i Intel)
  a AppImage a balíček `.deb` pro Linux. Na Macu a Linuxu se Codex spouští
  přes přihlašovací shell, takže se najde, i když je nainstalovaný přes
  Homebrew, npm nebo nvm, a widget si pamatuje, kde stál.
- **Zpětná vazba přímo z aplikace.** Tlačítko **Zpětná vazba** v hlavičce
  okna (nebo „Poslat zpětnou vazbu…“ v menu ikony) otevře formulář: vybereš,
  jestli jde o chybu, změnu, nebo vylepšení, napíšeš zprávu a můžeš přiložit
  snímek nebo log do 2 MB. Jméno a e-mail jsou nepovinné, bez nich se zpráva
  pošle anonymně. Okno předem ukáže, kam zpráva odejde a co se k ní přidá
  (verze a systém).

## 0.3.0 — 2026-10-28

### Přidáno

- **Nové jméno: Skylimit.** Postavičky se jmenují **Sunnie** (Claude)
  a **Dropie** (Codex).
- **Tlačítko Obnovit** ve widgetu vedle ozubeného kolečka. Hned přečte limity
  i spotřebu Codexu, záznamy Claude a logy T3 Code, místo aby čekalo na další
  pravidelné čtení.
- **Velký náhled nálad.** V záložce **Stavy** stačí najet myší na obrázek
  nálady a ukáže se velký animovaný náhled obou postaviček.

## 0.2.0 — 2026-10-28

### Přidáno

- **Limity Claude.** Skylimit je čte z logů T3 Code, kam Claude Agent SDK
  zapisuje události limitů. Přihlašovací údaje Claude se nečtou. Když pracuješ
  s Claude jinde (terminál, Claude Desktop) a T3 Code o tom neví, číslo dostane
  vlnovku jako „poslední známé“.
- **Záložka Stavy.** Všech osm nálad s vysvětlením, kdy která nastane, stavy
  řádků limitů, odkud se čísla berou a co znamenají barvy ikony.
- **Průhlednost widgetu** v nastavení (0–90 %). Průhledné je jen pozadí,
  text a postavičky zůstanou čitelné.

### Opraveno

- **Okno aplikace se přizpůsobí šířce.** Dlouhé názvy projektů se zkrátí
  a částky zůstanou v kartě, rozpady ukazují osm řádků a zbytek po rozbalení,
  nastavení je na širokém okně ve dvou sloupcích.

## 0.1.0 — 2026-10-28

### Přidáno

- **První verze.** Plovoucí widget s postavičkami, limity Codexu (5h a týden)
  a skóre měsíce v cenách API. Okno aplikace s rozpadem podle zdroje, modelu
  a projektu, sériemi dnů a nastavením paušálu, měny a průchodu kliknutí.
