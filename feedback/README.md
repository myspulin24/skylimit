# skylimit-feedback

Worker na Cloudflare, který přijímá zpětnou vazbu z okna **Zpětná vazba**
ve Skylimitu a zakládá z ní issue v soukromém repozitáři
`myspulin24/skylimit-feedback`. Přílohy ukládá do téhož repozitáře pod
`attachments/` a issue na ně odkazuje.

```
Skylimit (Rust)  ──POST /v1/feedback──▶  Worker  ──GitHub API──▶  soukromé repo
```

| Co | Kde |
| --- | --- |
| Adresa | `https://skylimit-feedback.michal-jasek.workers.dev/v1/feedback` |
| Cíl | proměnná `GITHUB_REPO` ve `wrangler.toml` |
| Token | tajemství `GITHUB_TOKEN` (nikdy v repozitáři, nikdy v aplikaci) |
| Limit | 5 odeslání za minutu z jedné adresy (`[[ratelimits]]`) |

Endpoint je veřejný. Aplikace žádné tajemství nenese: co je v instalačce, si
kdokoli vytáhne. Ochranou je pevný tvar zprávy s limity (`src/feedback.ts`),
omezení počtu odeslání a to, že všechno končí v soukromém repozitáři. IP adresa
se nikam neukládá. Zmínky `@někdo` v textu se zneškodní, aby přes zpětnou vazbu
nešlo nikoho obtěžovat.

## Token

Fine-grained personal access token jen na tohle jedno repo:

1. GitHub → **Settings → Developer settings → Personal access tokens →
   Fine-grained tokens → Generate new token**
   (<https://github.com/settings/personal-access-tokens/new>).
2. *Token name* `skylimit-feedback`, *Expiration* podle chuti. Po vypršení se
   zpětná vazba přestane ukládat a Worker hlásí 502.
3. *Repository access* → **Only select repositories** → `skylimit-feedback`.
4. *Permissions → Repository permissions*: **Issues: Read and write**,
   **Contents: Read and write** (Metadata: Read-only se přidá samo). Nic dalšího.
5. Vygenerovat, zkopírovat a v téhle složce:

   ```bash
   npx wrangler secret put GITHUB_TOKEN
   ```

   Wrangler se zeptá na hodnotu; token se nikde nevypíše ani neuloží na disk.

## Vývoj

```bash
cd feedback
# Místní tajemství pro `wrangler dev` (soubor je v .gitignore):
echo "GITHUB_TOKEN=..." > .dev.vars
npx wrangler dev --port 8787
```

Aplikaci pak na místní Worker přesměruje proměnná prostředí:

```bash
SKYLIMIT_FEEDBACK_URL=http://127.0.0.1:8787/v1/feedback npm run tauri dev
```

Testy běží s ostatními (`npm test`). GitHub v nich nahrazuje podvržený `fetch`,
takže nesahají na síť.

## Nasazení

```bash
cd feedback
npx wrangler deploy
npx wrangler tail     # živý log; chyby GitHubu se píšou sem, ne odesílateli
```
