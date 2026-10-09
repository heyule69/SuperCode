export function describeLink(href: string): { kind: 'external' | 'file' | 'anchor' | 'unsupported'; target: string; line?: number } {
  if (!href) return { kind: 'unsupported', target: '' };
  if (href.startsWith('#')) return { kind: 'anchor', target: href };
  if (/^(https?:|mailto:)/i.test(href)) {
    try { const url = new URL(href); return { kind: url.username || url.password ? 'unsupported' : 'external', target: url.href }; } catch { return { kind: 'unsupported', target: href }; }
  }
  let path = href;
  if (/^file:/i.test(path)) {
    try { const url = new URL(path); path = `${url.hostname ? `//${url.hostname}` : ''}${url.pathname}${url.hash}`; if (/^\/[a-z]:\//i.test(path)) path = path.slice(1); } catch { return { kind: 'unsupported', target: href }; }
  } else if (/^[a-z][\w+.-]*:/i.test(path) && !/^[a-z]:[\\/]/i.test(path)) return { kind: 'unsupported', target: href };
  const marker = /(?::(\d+)|#L?(\d+))$/i.exec(path);
  const line = marker ? Number(marker[1] ?? marker[2]) : undefined;
  try { return { kind: 'file', target: decodeURIComponent(marker ? path.slice(0, marker.index) : path), line: line && line > 0 ? line : undefined }; }
  catch { return { kind: 'unsupported', target: href }; }
}

export function resourceFromText(text: string) {
  const value = text.trim().replace(/^<(.+)>$/, '$1');
  if (!value || /[\r\n]/.test(value)) return null;
  if (/^(https?:\/\/|mailto:|file:)/i.test(value)) {
    const link = describeLink(value);
    return link.kind === 'file' || link.kind === 'external' ? link : null;
  }
  if (/^[a-z][\w+.-]*:/i.test(value) && !/^[a-z]:[\\/]/i.test(value)) return null;
  const file = describeLink(value);
  const path = file.target;
  if (/[<>|\r\n]/.test(path) || /&&/.test(path) || /^(?:cd|rm|ls|npm|npx|yarn|pnpm|git|curl|wget|python|node|bash|sh|powershell|cmd)\s/i.test(path)) return null;
  const explicitPath = /^(?:[a-z]:[\\/]|\/{1,2}|\.{1,2}[\\/])/i.test(path);
  const namedFile = /\.(?:[cm]?[jt]sx?|py|rs|go|java|c|cpp|h|cs|rb|php|sh|ps1|sql|json|toml|ya?ml|xml|ini|conf|lock|md|txt|log|csv|html?|css|scss|pdf|docx?|xlsx?|pptx?|png|jpe?g|gif|webp|svg|bmp|mp[34]|wav|zip|gz|tar)$/i.test(path) || /^(?:README|LICENSE|Dockerfile|Makefile)$/i.test(path);
  return file.kind === 'file' && (explicitPath || namedFile) ? file : null;
}

export const systemDocument = (path: string) => /\.(?:pdf|docx?|xlsx?|pptx?|od[tpfs]|rtf|txt|md|csv|png|jpe?g|gif|webp|svg|bmp|ico|mp[34]|wav|ogg|flac|mkv|mov|webm|html?)$/i.test(path);
