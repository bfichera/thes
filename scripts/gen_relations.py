#!/usr/bin/env python3
"""Extract WordNet 3.0 synset relations keyed by thes sense keys.

Usage: gen_relations.py <WordNet-3.0 dict dir> <en_thesaurus.jsonl> <out.tsv>

Output lines are `src_key<TAB>kind<TAB>dst_key` with kind one of
sim (similar to), see (also see), hyper (broader). Narrower terms are the
reverse of hyper and are derived at build time.
WordNet synsets are matched to jsonl senses by part of speech and gloss.
"""
import json
import re
import sys

KINDS = {"&": "sim", "^": "see", "@": "hyper", "@i": "hyper"}
FILES = {"n": "noun", "v": "verb", "a": "adj", "r": "adv"}
POS = {"noun": "n", "verb": "v", "adj": "a", "adv": "r"}


def lemma(w):
    return re.sub(r"\(\w+\)$", "", w).lower()


def main(d, jsonl, out):
    syn = {}
    by_gloss = {}
    for p, name in FILES.items():
        for line in open(f"{d}/data.{name}", encoding="utf-8"):
            if line.startswith("  "):
                continue
            head, gloss = line.rstrip("\n").split(" | ", 1)
            t = head.split()
            n = int(t[3], 16)
            words = {lemma(t[4 + 2 * i]) for i in range(n)}
            i = 4 + 2 * n
            ptrs = []
            for _ in range(int(t[i])):
                sym, tgt, tp, st = t[i + 1 : i + 5]
                i += 4
                if sym in KINDS and st == "0000":
                    ptrs.append((KINDS[sym], p if tp == "s" else tp, tgt))
            syn[(p, t[0])] = (words, ptrs)
            by_gloss.setdefault((p, gloss.strip()), []).append((p, t[0]))

    key_of = {}
    for line in open(jsonl, encoding="utf-8"):
        e = json.loads(line)
        p = POS[e["pos"]]
        cands = by_gloss.get((p, "; ".join(e["desc"]).strip()), [])
        words = {w.lower() for w in [e["word"], *e["synonyms"]]}
        cands = [c for c in cands if c not in key_of] or cands
        best = max(cands, key=lambda c: len(words & syn[c][0]), default=None)
        if best is not None and best not in key_of:
            key_of[best] = e["key"]

    rows = []
    for sid, (_, ptrs) in syn.items():
        if sid not in key_of:
            continue
        for kind, tp, tgt in ptrs:
            dst = key_of.get((tp, tgt))
            if dst:
                rows.append(f"{key_of[sid]}\t{kind}\t{dst}\n")
    with open(out, "w", encoding="utf-8") as f:
        f.writelines(rows)
    print(len(key_of), "synsets matched,", len(rows), "relations")


main(*sys.argv[1:4])
