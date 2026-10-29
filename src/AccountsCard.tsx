// Settings › Účty: which Claude and Codex account Skylimit reads. Claude's
// comes from Claude Code's own .claude.json, Codex's from the app-server
// (src-tauri/src/commands.rs `accounts`). Shown here only, never in the
// widget; refreshed while Settings are open, since Codex answers a moment
// after the app starts and a login can change.

import { useEffect, useState, type ReactNode } from "react";

import { getAccounts } from "./api";
import { Group } from "./controls";
import {
  billingLabel,
  claudePlanLabel,
  claudeRoleLabel,
  claudeTierLabel,
  codexCreditsLabel,
  codexKindLabel,
  codexPlanLabel,
  extraUsageLabel,
  formatAgo,
  formatDate,
  formatResets,
  formatTokens,
} from "./format";
import type { Accounts, ClaudeAccount, CodexAccount, Provider } from "./types";

const REFRESH_MS = 20_000;

export function AccountsCard() {
  const [accounts, setAccounts] = useState<Accounts | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    const load = () =>
      getAccounts().then(
        (a) => {
          if (!alive) return;
          setAccounts(a);
          setError(null);
        },
        (err: unknown) => alive && setError(String(err)),
      );
    void load();
    const timer = window.setInterval(() => void load(), REFRESH_MS);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, []);

  const now = Date.now() / 1000;
  return (
    <Group
      title="Účty"
      footer="Skylimit účty jen čte, a to lokálně: Claude ze souboru .claude.json, kam si ho zapisuje Claude Code, Codex z codex app-serveru. Nikam je neposílá."
    >
      {error && !accounts ? (
        <p className="row-desc err-text account-error" role="alert">
          {error}
        </p>
      ) : (
        <div className="accounts">
          <ClaudePanel account={accounts?.claude} loading={accounts === null} now={now} />
          <CodexPanel account={accounts?.codex} note={accounts?.codexNote ?? null} loading={accounts === null} now={now} />
        </div>
      )}
    </Group>
  );
}

type Fact = [string, ReactNode | null | undefined];

function Panel({
  provider,
  name,
  plan,
  email,
  sub,
  facts,
  source,
  raw,
  empty,
}: {
  provider: Provider;
  name: string;
  plan?: string | null;
  email?: string | null;
  sub?: string | null;
  facts?: Fact[];
  source?: string;
  raw?: Fact[];
  empty?: ReactNode;
}) {
  const shown = (facts ?? []).filter((f): f is [string, ReactNode] => f[1] !== null && f[1] !== undefined && f[1] !== "");
  const rawShown = (raw ?? []).filter((f): f is [string, ReactNode] => f[1] !== null && f[1] !== undefined && f[1] !== "");
  return (
    <article className="account" aria-label={`Účet ${name}`}>
      <header className="account-head">
        <i className={`dot dot-${provider}`} aria-hidden="true" />
        <span className="account-provider">{name}</span>
        {plan && <span className="account-plan">{plan}</span>}
      </header>
      {empty ? (
        <p className="account-empty">{empty}</p>
      ) : (
        <>
          <p className="account-email">{email ?? "e-mail neznámý"}</p>
          {sub && <p className="account-sub">{sub}</p>}
          {shown.length > 0 && (
            <dl className="account-facts">
              {shown.map(([k, v]) => (
                <div key={k}>
                  <dt>{k}</dt>
                  <dd>{v}</dd>
                </div>
              ))}
            </dl>
          )}
          {rawShown.length > 0 && (
            <details className="account-raw">
              <summary>Technické údaje</summary>
              <dl className="account-facts">
                {rawShown.map(([k, v]) => (
                  <div key={k}>
                    <dt>{k}</dt>
                    <dd className="key">{v}</dd>
                  </div>
                ))}
              </dl>
            </details>
          )}
        </>
      )}
      {source && <p className="account-source">{source}</p>}
    </article>
  );
}

function ClaudePanel({ account, loading, now }: { account: ClaudeAccount | null | undefined; loading: boolean; now: number }) {
  if (!account) {
    return (
      <Panel
        provider="claude"
        name="Claude"
        empty={
          loading
            ? "Zjišťuji účet…"
            : "Claude Code tu není přihlášený. Přihlas se v něm (claude, pak /login) a účet se tu ukáže."
        }
      />
    );
  }
  const plan = claudePlanLabel(account.organizationType);
  const tier = claudeTierLabel(account.rateLimitTier);
  const subscription = [
    account.subscriptionCreatedAt !== null ? `od ${formatDate(account.subscriptionCreatedAt)}` : null,
    billingLabel(account.billing),
  ]
    .filter(Boolean)
    .join(", ");
  const source = [
    "Z Claude Code",
    account.profileFetchedAt !== null ? `profil ${formatAgo(now - account.profileFetchedAt)}` : null,
    account.limitsFetchedAt !== null ? `limity ${formatAgo(now - account.limitsFetchedAt)}` : null,
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <Panel
      provider="claude"
      name="Claude"
      plan={plan}
      email={account.email}
      sub={[account.name, account.organization].filter(Boolean).join(" · ") || null}
      facts={[
        ["Role", claudeRoleLabel(account.role)],
        // A Team or Enterprise seat with Max-sized limits says so in its tier.
        ["Limity", tier && plan !== tier ? `jako ${tier}` : null],
        ["Předplatné", subscription],
        ["Extra využití", extraUsageLabel(account.extraUsage, account.extraUsageBlocked)],
        ["Účet založen", account.accountCreatedAt !== null ? formatDate(account.accountCreatedAt) : null],
      ]}
      raw={[
        ["organizationType", account.organizationType],
        ["seatTier", account.seatTier],
        ["userRateLimitTier", account.rateLimitTier],
        ["billingType", account.billing],
      ]}
      source={source}
    />
  );
}

function CodexPanel({
  account,
  note,
  loading,
  now,
}: {
  account: CodexAccount | null | undefined;
  note: string | null;
  loading: boolean;
  now: number;
}) {
  if (!account || account.kind === "none") {
    let empty: string;
    if (loading) empty = "Zjišťuji účet…";
    else if (account) empty = "Codex CLI není přihlášený. Přihlas se příkazem codex login a účet se tu ukáže.";
    else if (note) empty = `${note}. Účet se ukáže, až Codex CLI odpoví.`;
    else empty = "Čekám, až se ozve Codex CLI…";
    return <Panel provider="codex" name="Codex" empty={empty} source={account?.cliVersion ? `Codex CLI ${account.cliVersion}` : undefined} />;
  }
  const source = [`Z Codex CLI${account.cliVersion ? ` ${account.cliVersion}` : ""}`, `načteno ${formatAgo(now - account.readAt)}`].join(" · ");
  return (
    <Panel
      provider="codex"
      name="Codex"
      plan={codexPlanLabel(account.plan)}
      email={account.email ?? (account.kind === "chatgpt" ? null : codexKindLabel(account.kind))}
      sub={account.workspace}
      facts={[
        ["Přihlášení", codexKindLabel(account.kind)],
        ["Resety limitu", account.resetCredits !== null ? `${formatResets(account.resetCredits)} k dispozici` : null],
        ["Kredity", codexCreditsLabel(account.credits)],
        ["Tokenů celkem", account.lifetimeTokens !== null ? formatTokens(account.lifetimeTokens) : null],
      ]}
      raw={[
        ["type", account.kind],
        ["planType", account.plan],
      ]}
      source={source}
    />
  );
}
