import { useEffect, useState } from "react";
import { useT } from "../i18n";
import { Account, AccountDetails, accountDetails } from "../lib/api";
import { fmtDate, fmtRelative, fmtTime, hue } from "../lib/format";
import { accountStatus, canSwitch, who } from "../lib/status";
import Icon from "./Icon";
import Modal from "./Modal";
import { Alert, Avatar, Field, StatusPill } from "./ui";

interface Props {
  account: Account;
  onClose: () => void;
  onSwitch: (a: Account) => void;
  onDelete: (a: Account) => void;
}

export default function AccountDetailsDialog({ account, onClose, onSwitch, onDelete }: Props) {
  const t = useT();
  const [details, setDetails] = useState<AccountDetails | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");

  useEffect(() => {
    let alive = true;
    setLoading(true);
    accountDetails(account.id)
      .then((d) => alive && setDetails(d))
      .catch((e) => alive && setError(String(e)))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
  }, [account.id]);

  const s = accountStatus(account, t);
  const d = details;
  const where = account.active ? t.common.currentAccount : account.on_desktop ? t.details.onDesktop : t.common.vaultOnly;

  return (
    <Modal
      title={t.details.title}
      onClose={onClose}
      size="lg"
      footer={
        <>
          <button className="btn ghost dangerText" onClick={() => onDelete(account)}>
            <Icon name="trash" size={14} />
            {t.common.delete}
          </button>
          <span className="spacer" />
          <button className="btn ghost" onClick={onClose}>
            {t.common.close}
          </button>
          {canSwitch(account) && (
            <button className="btn primary" onClick={() => onSwitch(account)}>
              <Icon name="swap" size={14} />
              {t.details.switchTo}
            </button>
          )}
        </>
      }
    >
      <div className="detailHero">
        <Avatar name={who(account)} hue={hue(account.id)} size={48} />
        <div className="detailWho">
          <b>{who(account)}</b>
          <span>{account.email || account.id}</span>
        </div>
        <div className="detailTags">
          <StatusPill tone={s.tone} label={s.label} />
          <span className="tag">{where}</span>
        </div>
      </div>

      {error && <Alert tone="bad">{error}</Alert>}
      {loading && !d && (
        <div className="detailGrid">
          <div className="skeleton" />
          <div className="skeleton" />
          <div className="skeleton short" />
        </div>
      )}

      {d && (
        <div className="detailGrid">
          <section className="fieldGroup">
            <h3>{t.details.account}</h3>
            <Field label={t.details.name} value={d.display_name} />
            <Field label={t.details.email} value={d.email} copy />
            <Field label={t.details.role} value={d.role} />
            <Field label={t.details.awsAccount} value={d.aws_account_id} mono copy />
            <Field label={t.details.accountId} value={d.id} mono copy />
          </section>

          <section className="fieldGroup">
            <h3>{t.details.signIn}</h3>
            <Field label={t.details.accessToken}>
              {d.access_expires_at ? t.details.withRelative(fmtDate(d.access_expires_at), fmtRelative(d.access_expires_at, t)) : "—"}
            </Field>
            <Field label={t.details.validUntil}>
              {d.refresh_expires_at ? t.details.withRelative(fmtDate(d.refresh_expires_at), fmtRelative(d.refresh_expires_at, t)) : "—"}
            </Field>
            <Field label={t.details.tenant} value={d.tenant_url} mono copy />
            <Field label="issuer" value={d.issuer_url} mono copy />
            <Field label="client_id" value={d.client_id} mono copy />
            {d.flag && <Field label={t.details.flag} value={d.flag === "auth_invalid" ? t.details.flagAuthInvalid : d.flag === "forbidden" ? t.status.forbidden : d.flag} />}
          </section>

          <section className="fieldGroup">
            <h3>{t.details.deviceAndRecords}</h3>
            <Field label={t.details.deviceId} value={d.device_id} mono copy />
            <Field label={t.details.createdAt} value={fmtTime(d.created_at)} />
            <Field label={t.details.lastUsed} value={fmtTime(d.last_used_at)} />
            <Field label={t.details.addedAt} value={fmtTime(d.added_at)} />
            <Field label={t.details.lastSynced} value={fmtTime(d.last_synced_at)} />
          </section>
        </div>
      )}
    </Modal>
  );
}
