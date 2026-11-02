// Keep executable/reference content out of translation requests.
const tokenPattern = /⟪SC\d+⟫/g;
const protectedPattern = /^ {0,3}(`{3,}|~{3,})[^\n]*\n[\s\S]*?(?:^ {0,3}\1[ \t]*(?=\n|$)|$(?![\s\S]))|`+[^`\n]+`+|!\[[^\]\n]*\]\([^\n]*?\)|\]\(\s*(?:<[^>\n]+>|[^()\s]+(?:\([^\n)]*\)[^()\s]*)*)(?:\s+["'][^"'\n]*["'])?\s*\)|^ {0,3}\[[^\]\n]+\]:[^\n]*|https?:\/\/[^\s<>]+|\b[A-Za-z]:[\\/][^\s<>"`]+|(?:^|(?<=\s))\/(?:[^\s/]+\/)*[^\s/]+|^(?:\$ )?(?:npm|pnpm|yarn|cargo|git|python3?|node|powershell|curl)\b[^\n]*|⟪SC\d+⟫/gm;

export function maskTranslation(text: string) {
  const values: string[] = [];
  const masked = text.replace(protectedPattern, value => `⟪SC${values.push(value) - 1}⟫`);
  return { masked, values };
}

export function needsTranslation(text: string): boolean {
  const plain = maskTranslation(text).masked.replace(tokenPattern, '').replace(/[*#_>|\d\-]/g, ' ');
  const han = (plain.match(/\p{Script=Han}/gu) ?? []).length;
  const foreign = (plain.match(/[\p{L}]/gu) ?? []).filter(char => !/\p{Script=Han}/u.test(char));
  if (!foreign.length) return false;
  const nonLatin = foreign.filter(char => !/\p{Script=Latin}/u.test(char)).length;
  if (nonLatin > 1 && nonLatin > han / 2) return true;
  const words = plain.match(/[\p{Script=Latin}]+(?:['’][\p{Script=Latin}]+)*/gu) ?? [];
  if (han && words.every(word => /^[A-Z]/.test(word))) return false;
  if (han && foreign.length < han * 2) return false;
  return words.length >= 3 || words.length >= 2 && (/[.!?。！？]/.test(plain) || !han && words.some(word => /^[a-zà-ɏ]/.test(word)))
    || !han && /^(?:done|hello|thanks|thank you|yes|no|okay|ok|completed|sorry)[.!\s]*$/i.test(plain.trim());
}

export interface TranslationPart { text: string; translate: boolean }
export function translationParts(masked: string, streaming: boolean): TranslationPart[] {
  // While streaming, only completed paragraphs are immutable enough to translate.
  const end = streaming ? masked.lastIndexOf('\n\n') + 2 : masked.length;
  const stableEnd = streaming && end === 1 ? 0 : end;
  const parts: TranslationPart[] = [];
  let offset = 0;
  while (offset < stableEnd) {
    const rest = masked.slice(offset, stableEnd);
    const blank = /^\s+/.exec(rest);
    if (blank) { parts.push({ text: blank[0], translate: false }); offset += blank[0].length; continue; }
    const paragraph = rest.indexOf('\n\n');
    let size = Math.min(paragraph < 0 ? rest.length : paragraph, 2400);
    if (size < rest.length && size === 2400) {
      const boundary = rest.slice(0, size).search(/\s+\S*$/);
      if (boundary > 1200) size = boundary;
      // Never split a protected token or UTF-16 surrogate pair.
      const tokenStart = rest.lastIndexOf('⟪', size - 1);
      if (tokenStart >= 0 && rest.indexOf('⟫', tokenStart) >= size) size = tokenStart || rest.indexOf('⟫', tokenStart) + 1;
      if (/[\uD800-\uDBFF]/.test(rest[size - 1])) size--;
    }
    const text = rest.slice(0, size);
    parts.push({ text, translate: needsTranslation(text) });
    offset += size;
  }
  if (offset < masked.length) parts.push({ text: masked.slice(offset), translate: false });
  return parts;
}

export function validateTranslation(source: string, result: unknown): string {
  if (typeof result !== 'string' || result.length > 24576 || !result.trim()) throw new Error('翻译结果为空或过长');
  const expected = source.match(tokenPattern) ?? [];
  const actual = result.match(tokenPattern) ?? [];
  if (JSON.stringify(expected) !== JSON.stringify(actual)) throw new Error('译文未保留代码或链接，已显示原文');
  const plain = result.replace(tokenPattern, '');
  if (maskTranslation(plain).values.length || !/\p{Script=Han}/u.test(plain)) throw new Error('译文格式不完整，已显示原文');
  return result.trim();
}

export function restoreTranslation(text: string, values: string[]): string {
  return text.replace(tokenPattern, token => values[Number(token.slice(3, -1))] ?? token);
}
