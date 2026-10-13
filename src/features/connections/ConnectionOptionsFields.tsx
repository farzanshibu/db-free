// SOT: connection-options-fields, mongo-srv-toggle, auth-database-field, uri-parameters-field
import type { ConnectionFieldIssue, ConnectionInput } from "@/lib/bindings";
import { paramValue, withParam } from "@/lib/connectionUri";
import { engineMeta, supportsSrv, usesUriParams } from "@/lib/engines";
import { Field, Toggle } from "@/components/global/Field";
import { issueFor } from "./formHelpers";
import { FieldIssue } from "./FieldIssue";

interface ConnectionOptionsFieldsProps {
  input: ConnectionInput;
  patch: (partial: Partial<ConnectionInput>) => void;
  issues: readonly ConnectionFieldIssue[];
}

// WHAT:  The server-form fields a connection string carries but the host / port /
//        user / password grid cannot: MongoDB's SRV switch and authentication
//        database, and the raw driver options (`replicaSet`, `application_name`,
//        `options`, `pgbouncer`…).
// WHY:   Pasting `mongodb+srv://…/db?authSource=admin&replicaSet=rs0` used to
//        keep only the host. The rest of the string is part of the connection:
//        dropping `authSource` fails the login, dropping `replicaSet` changes
//        which server answers. The options are stored with the connection and
//        handed to the driver that understands them.
// HOW:   Options are one `key=value&key=value` string (`extra.uriParams`); the
//        authentication database is a view onto its `authSource` entry, so the
//        two fields can never disagree. Secrets do not belong here: Rust refuses
//        a `password=` parameter, and the password field is sealed.
// WHERE: src-tauri/src/model/connection_org.rs (ConnectionExtras), src-tauri/src/integrations/mongodb.rs
export function ConnectionOptionsFields({ input, patch, issues }: ConnectionOptionsFieldsProps) {
  const meta = engineMeta(input.engine);
  const extra = input.extra;
  const setParams = (uriParams: string | null) => { patch({ extra: { ...extra, uriParams } }); };
  const isMongo = input.engine === "mongodb";
  const authSource = paramValue((extra.uriParams ?? "").split("&"), "authSource") ?? "";
  if (!supportsSrv(input.engine) && !usesUriParams(input.engine)) return null;

  return (
    <div className="flex flex-col gap-3">
      {supportsSrv(input.engine) ? (
        <Toggle
          checked={extra.srv}
          onChange={(srv) => { patch({ extra: { ...extra, srv }, port: srv ? null : meta.defaultPort }); }}
          label="Use SRV record (mongodb+srv://)"
          description="For Atlas and other clusters addressed by a DNS name: the member list comes from DNS, so the host takes no port and TLS is on."
        />
      ) : null}
      {isMongo ? (
        <Field
          label="Authentication database"
          value={authSource}
          onChange={(value) => { setParams(withParam(extra.uriParams, "authSource", value.trim())); }}
          placeholder="admin"
          optional
          description="Where the user was created. Leave empty to use the database above (or admin when that is empty)."
        />
      ) : null}
      {usesUriParams(input.engine) ? (
        <div className="flex flex-col gap-1">
          <Field
            label="Connection options"
            value={extra.uriParams ?? ""}
            onChange={(value) => { setParams(value.trim() === "" ? null : value); }}
            placeholder={isMongo ? "replicaSet=rs0&readPreference=secondaryPreferred" : "application_name=db-free&options=-c%20search_path%3Dapp"}
            optional
            mono
            description="Extra driver parameters as key=value pairs joined by &. Stored unencrypted: put passwords in the password field."
          />
          <FieldIssue message={issueFor(issues, "uri_params")} />
        </div>
      ) : null}
    </div>
  );
}
