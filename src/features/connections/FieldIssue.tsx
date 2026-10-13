// SOT: field-issue-message, inline-field-error
// WHAT:  The message under a form field that failed validation, or nothing.
// WHY:   Rust names the field each problem belongs to (`validate_connection`);
//        painting it under that field beats one banner for the first failure.
//        The shared `Field` has no error slot, so the message is a sibling.
export function FieldIssue({ message }: { message: string | null }) {
  if (message === null) return null;
  return (
    <p role="alert" className="-mt-0.5 text-xs text-danger">
      {message}
    </p>
  );
}
