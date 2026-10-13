// SOT: workflow-notification-form, workflow-email-form, workflow-slack-form
import type { SmtpSecurity } from "@/lib/bindings";
import { AppSelect, Field, NumberInput } from "@/components/global/Field";
import { Icon } from "@/lib/icons";
import { Button } from "@/components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { EMAIL_TEMPLATES } from "../snippets";
import { SecretField, TemplateHint, type FormProps } from "./common";

export function NotificationForm({ config, onChange }: FormProps<"notification">) {
  return (
    <div className="flex flex-col gap-2">
      <Field label="Title" value={config.title} placeholder="{{rows.count}} new signups" onChange={(title) => { onChange({ ...config, title }); }} />
      <div className="flex flex-col gap-1.5">
        <Label>Body</Label>
        <Textarea value={config.body} rows={4} className="font-sans" placeholder="{{name}} just signed up" onChange={(event) => { onChange({ ...config, body: event.target.value }); }} />
      </div>
      <TemplateHint>With no input rows the placeholders render empty, so a test run still shows something.</TemplateHint>
    </div>
  );
}

const SECURITY: readonly { value: SmtpSecurity; label: string }[] = [
  { value: "auto", label: "Automatic (TLS on 465, STARTTLS otherwise)" },
  { value: "implicit", label: "TLS from the start (SMTPS)" },
  { value: "start_tls", label: "STARTTLS required" },
  { value: "none", label: "None (local test server only)" },
];

// WHAT:  The Email node: SMTP login, sender, and a templated message.
// WHY:   The password is sealed by the core on save and never shown again. A new
//        node starts from the last Email setup you saved (host, port, login, sender).
export function EmailForm({ config, onChange }: FormProps<"email">) {
  return (
    <div className="flex flex-col gap-2">
      <div className="flex gap-2">
        <Field label="SMTP host" mono value={config.host} placeholder="smtp.example.com" onChange={(host) => { onChange({ ...config, host }); }} />
        <div className="w-24 shrink-0">
          <NumberInput integer label="Port" value={config.port} onChange={(next) => { onChange({ ...config, port: Math.min(65535, Math.max(0, Math.round(next ?? 0))) }); }} />
        </div>
      </div>
      <AppSelect label="Connection security" value={config.security} options={SECURITY} onChange={(security) => { onChange({ ...config, security }); }} />
      <Field label="User name" optional value={config.username} onChange={(username) => { onChange({ ...config, username }); }} />
      <SecretField label="Password" secret={config.password} placeholder="SMTP password" onChange={(password) => { onChange({ ...config, password }); }} />
      <Field label="From" value={config.from} placeholder="Alerts <alerts@example.com>" onChange={(from) => { onChange({ ...config, from }); }} />
      <Field label="To" value={config.to} placeholder="team@example.com, {{email}}" description="Several addresses may be separated by commas." onChange={(to) => { onChange({ ...config, to }); }} />
      <Field label="Subject" optional value={config.subject} onChange={(subject) => { onChange({ ...config, subject }); }} />
      <div className="flex flex-col gap-1.5">
        <div className="flex items-center justify-between">
          <Label>Body</Label>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="ghost" size="xs">
                <Icon name="sparkles" size={12} />
                Templates
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-72">
              {EMAIL_TEMPLATES.map((template) => (
                <DropdownMenuItem key={template.label} onSelect={() => { onChange({ ...config, subject: template.subject, body: template.body }); }}>
                  <span className="flex flex-col">
                    <span className="text-xs">{template.label}</span>
                    <span className="text-[11px] text-muted">{template.description}</span>
                  </span>
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
        <Textarea value={config.body} rows={8} spellCheck={false} onChange={(event) => { onChange({ ...config, body: event.target.value }); }} />
        <TemplateHint>
          A body with an HTML tag is sent as HTML, anything else as plain text. <code className="font-mono text-foreground">{"{{rows.html}}"}</code> suits HTML,{" "}
          <code className="font-mono text-foreground">{"{{rows.table}}"}</code> plain text; they list at most 50 rows.
        </TemplateHint>
      </div>
    </div>
  );
}

export function SlackForm({ config, onChange }: FormProps<"slack">) {
  return (
    <div className="flex flex-col gap-2">
      <SecretField label="Webhook URL" secret={config.webhook} placeholder="https://hooks.slack.com/services/…" onChange={(webhook) => { onChange({ ...config, webhook }); }} />
      <div className="flex flex-col gap-1.5">
        <Label>Message</Label>
        <Textarea value={config.text} rows={5} className="font-sans" placeholder={"{{rows.count}} new signups today:\n{{rows.table}}"} onChange={(event) => { onChange({ ...config, text: event.target.value }); }} />
      </div>
      <TemplateHint>
        <code className="font-mono text-foreground">{"{{rows.table}}"}</code>, <code className="font-mono text-foreground">{"{{rows.json}}"}</code> and{" "}
        <code className="font-mono text-foreground">{"{{rows.list}}"}</code> post up to 50 rows; Slack draws the code blocks in a fixed-width font. Add a LIMIT upstream for a different size.
      </TemplateHint>
    </div>
  );
}
