const SECRET_KEY_LINE = /secret_key\s*=\s*"[^"]*"/g;
const RELAY_TOKEN_LINE = /relay_token\s*=\s*"[^"]*"/g;

/** Masks `secret_key`/`relay_token` assignments and every registered secret value in text that is printed to the job log. */
export function redact(text: string, secrets: readonly string[]): string {
  let out = text.replace(SECRET_KEY_LINE, 'secret_key = "***"').replace(RELAY_TOKEN_LINE, 'relay_token = "***"');
  for (const s of secrets) if (s !== '') out = out.split(s).join('***');
  return out;
}
