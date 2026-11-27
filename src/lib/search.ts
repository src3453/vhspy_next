// パラメータパネルの曖昧検索。
// 全角・半角、大文字・小文字、カタカナ・ひらがな、空白や記号の違いを無視し、
// 部分一致 > 飛び飛びの一致 (サブシーケンス) > 1 文字違い (英数字) の順に点を付ける。

/** 正規化した文字列と、その各文字が元の文字列の何文字目かの対応 */
type Norm = { s: string; map: number[] };

// 区切りとして無視する文字 (長音「ー」は意味があるので残す)
const IGNORED = /[\s・･\-_./\\()（）「」『』\[\]【】,，、。:：;；'"`]/u;

function normChar(ch: string): string {
  let c = ch.normalize("NFKC").toLowerCase();
  // カタカナ → ひらがな
  c = c.replace(/[ァ-ヶ]/g, (k) => String.fromCharCode(k.charCodeAt(0) - 0x60));
  return c;
}

const cache = new Map<string, Norm>();

export function normalize(text: string): Norm {
  const hit = cache.get(text);
  if (hit) return hit;
  let s = "";
  const map: number[] = [];
  let i = 0;
  for (const ch of text) {
    if (!IGNORED.test(ch)) {
      for (let c of normChar(ch)) {
        if (IGNORED.test(c)) continue;
        // 半角カナの濁点・半濁点 (ｶﾞ → カ + 結合濁点) は前の文字とくっつける
        if ((c === "゙" || c === "゚") && s.length > 0) {
          c = (s[s.length - 1] + c).normalize("NFC");
          if (c.length === 1) {
            s = s.slice(0, -1) + c;
            continue;
          }
        }
        s += c;
        map.push(i);
      }
    }
    i += ch.length;
  }
  const n = { s, map };
  if (cache.size > 5000) cache.clear();
  cache.set(text, n);
  return n;
}

/** 検索語を空白で区切って正規化する (すべての語が一致したものだけを残す AND 検索) */
export function parseQuery(q: string): string[] {
  return q
    .split(/[\s　]+/)
    .map((t) => normalize(t).s)
    .filter((t) => t.length > 0);
}

type Hit = { score: number; idx: number[] };

const isWordStart = (s: string, i: number) => i === 0 || !/[a-z0-9]/.test(s[i - 1]) || !/[a-z0-9]/.test(s[i]);

/** 1 語と 1 つの文字列の一致。fuzzy=false なら部分一致だけ見る */
function matchToken(tok: string, field: Norm, fuzzy: boolean): Hit | null {
  const s = field.s;
  if (!s) return null;

  // 部分一致
  const at = s.indexOf(tok);
  if (at >= 0) {
    let score = 100 + (at === 0 ? 30 : isWordStart(s, at) ? 15 : 0) + (tok.length === s.length ? 20 : 0);
    return { score, idx: range(at, tok.length).map((k) => field.map[k]) };
  }
  if (!fuzzy) return null;

  // サブシーケンス (飛び飛びの一致)。間が空きすぎるものは捨てる
  if (tok.length >= 2) {
    const idx: number[] = [];
    let j = 0;
    let gaps = 0;
    let first = -1;
    for (let k = 0; k < s.length && j < tok.length; k++) {
      if (s[k] === tok[j]) {
        if (first < 0) first = k;
        else if (idx.length && k !== idx[idx.length - 1] + 1) gaps++;
        idx.push(k);
        j++;
      }
    }
    if (j === tok.length) {
      const span = idx[idx.length - 1] - first + 1;
      if (span <= tok.length * 2 + 2 && gaps <= Math.ceil(tok.length / 2)) {
        return { score: 60 - gaps * 8 - (span - tok.length) * 2, idx: idx.map((k) => field.map[k]) };
      }
    }
  }

  // 英数字の打ち間違い (挿入・削除・置換・隣接入れ替えのどれか 1 つ)
  if (tok.length >= 4 && /^[a-z0-9]+$/.test(tok)) {
    for (let len = tok.length - 1; len <= tok.length + 1; len++) {
      for (let k = 0; k + len <= s.length; k++) {
        if (editDistance1(tok, s.slice(k, k + len))) {
          return { score: 35, idx: range(k, len).map((m) => field.map[m]) };
        }
      }
    }
  }
  return null;
}

function range(start: number, n: number) {
  return Array.from({ length: n }, (_, i) => start + i);
}

/** 編集距離 (隣接入れ替えを含む) が 1 以下か */
function editDistance1(a: string, b: string): boolean {
  if (a === b) return true;
  const la = a.length;
  const lb = b.length;
  if (Math.abs(la - lb) > 1) return false;
  let i = 0;
  while (i < la && i < lb && a[i] === b[i]) i++;
  if (la === lb) {
    if (a.slice(i + 1) === b.slice(i + 1)) return true; // 置換
    return a[i] === b[i + 1] && a[i + 1] === b[i] && a.slice(i + 2) === b.slice(i + 2); // 入れ替え
  }
  return la > lb ? a.slice(i + 1) === b.slice(i) : a.slice(i) === b.slice(i + 1);
}

/** 検索対象の項目。label はハイライト表示に使う */
export type SearchFields = {
  label: string;
  /** ラベルと同じくらい強く一致させる短い文字列 (パス、グループ名など) */
  keys?: string[];
  /** 弱く一致させる長い文字列 (説明など)。部分一致だけ見る */
  text?: string[];
};

export type SearchResult = { score: number; labelHits: number[] };

/** すべての語が一致すれば点数とラベル中の一致位置を返す */
export function search(tokens: string[], f: SearchFields): SearchResult | null {
  if (tokens.length === 0) return { score: 0, labelHits: [] };
  const label = normalize(f.label);
  const keys = (f.keys ?? []).map(normalize);
  const text = (f.text ?? []).map(normalize);
  let total = 0;
  const hits = new Set<number>();
  for (const tok of tokens) {
    let best = 0;
    let bestLabel: number[] | null = null;
    const l = matchToken(tok, label, true);
    if (l) {
      best = l.score * 1.0;
      bestLabel = l.idx;
    }
    for (const k of keys) {
      const h = matchToken(tok, k, true);
      if (h && h.score * 0.7 > best) {
        best = h.score * 0.7;
        bestLabel = null;
      }
    }
    if (best < 70) {
      for (const t of text) {
        const h = matchToken(tok, t, false);
        if (h && h.score * 0.4 > best) {
          best = h.score * 0.4;
          bestLabel = null;
        }
      }
    }
    if (best <= 0) return null;
    // ラベル以外で一番良く一致しても、ラベル中に一致があれば強調表示はしておく
    if (!bestLabel && l) bestLabel = l.idx;
    bestLabel?.forEach((k) => hits.add(k));
    total += best;
  }
  return { score: total / tokens.length, labelHits: [...hits].sort((a, b) => a - b) };
}

/** ラベルを一致部分とそれ以外に分ける (強調表示用) */
export function segments(label: string, hits: number[]): { t: string; hit: boolean }[] {
  if (hits.length === 0) return [{ t: label, hit: false }];
  const set = new Set(hits);
  const out: { t: string; hit: boolean }[] = [];
  let i = 0;
  for (const ch of label) {
    const hit = set.has(i);
    const last = out[out.length - 1];
    if (last && last.hit === hit) last.t += ch;
    else out.push({ t: ch, hit });
    i += ch.length;
  }
  return out;
}
