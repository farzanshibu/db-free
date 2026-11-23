// SOT: tls-files-section, custom-ca-and-client-certificate-form
import type { ConnectionField, ConnectionFieldIssue, ConnectionInput } from "@/lib/bindings";
import { tlsFileSupport } from "@/lib/engines";
import { pickCertificateFile } from "@/lib/connectionFiles";
import { useState } from "react";
import { Disclosure } from "@/components/global/Disclosure";
import { Field } from "@/components/global/Field";
import { Button } from "@/components/ui/button";
import { issueFor } from "./formHelpers";
import { FieldIssue } from "./FieldIssue";

interface TlsFilesSectionProps {
  input: ConnectionInput;
  patch: (partial: Partial<ConnectionInput>) => void;
  issues: readonly ConnectionFieldIssue[];
}

type TlsKey = "caPath" | "certPath" | "keyPath";

// WHAT:  A private CA and, for mutual TLS, a client certificate and key.
// WHY:   Self-hosted servers sign with their own CA and managed ones (RDS, Cloud
//        SQL, Azure) publish a bundle; checking the server against it needs a
//        file the OS trust store does not have. Only the *path* is saved — the
//        file is read when a session opens, so a rotated certificate needs no
//        re-save.
// HOW:   Shows exactly what the engine's adapter applies (`tlsFileSupport`):
//        PostgreSQL and MySQL take all three, MongoDB a CA plus one PEM holding
//        certificate and key, SQL Server a CA only. Other engines show nothing,
//        rather than a field that would be saved and ignored.
// WHERE: src-tauri/src/integrations/{postgres,mysql,mssql,mongodb}.rs
export function TlsFilesSection({ input, patch, issues }: TlsFilesSectionProps) {
  const support = tlsFileSupport(input.engine);
  const tls = input.extra.tls;
  const [open, setOpen] = useState(tls.caPath !== null || tls.certPath !== null || tls.keyPath !== null);
  if (support === "none") return null;
  const hasIssue = issueFor(issues, "tls_ca_path") !== null || issueFor(issues, "tls_cert_path") !== null || issueFor(issues, "tls_key_path") !== null;
  const count = support === "mutual" ? 3 : support === "bundle" ? 2 : 1;
  const set = (key: TlsKey, value: string) => {
    patch({ extra: { ...input.extra, tls: { ...tls, [key]: value.trim() === "" ? null : value } } });
  };
  const browse = async (key: TlsKey, title: string) => {
    const path = await pickCertificateFile(title);
    if (path !== null) set(key, path);
  };
  const row = (key: TlsKey, label: string, title: string, field: ConnectionField, description?: string) => (
    <div className="flex flex-col gap-1">
      <Field
        label={label}
        value={tls[key] ?? ""}
        onChange={(value) => { set(key, value); }}
        placeholder="/path/to/file.pem"
        optional
        mono
        tip={description}
        suffix={
          <Button variant="secondary" size="sm" onClick={() => void browse(key, title)} className="rounded-md">
            Browse…
          </Button>
        }
      />
      <FieldIssue message={issueFor(issues, field)} />
    </div>
  );

  return (
    <Disclosure
      label="Certificate files"
      count={count}
      tip="Optional. Pair a CA file with “Verify CA” or “Verify full” to check the server against it."
      open={open || hasIssue}
      onOpenChange={setOpen}
      className="gap-3"
    >
      {row("caPath", "CA certificate", "Choose the CA certificate", "tls_ca_path", "The authority that signed the server's certificate (PEM).")}
      {support === "mutual"
        ? row("certPath", "Client certificate", "Choose the client certificate", "tls_cert_path", "Presented to servers that require mutual TLS (PEM).")
        : null}
      {support === "mutual" ? row("keyPath", "Client key", "Choose the client key", "tls_key_path", "The private key for the client certificate (PEM, unencrypted).") : null}
      {support === "bundle" ? row("certPath", "Client certificate and key", "Choose the client PEM", "tls_cert_path", "One PEM file holding the client certificate followed by its private key.") : null}
    </Disclosure>
  );
}
