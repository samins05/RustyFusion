"""GCRA fire limit + projectile expiry benchmark, read from shard.log.

usage: python tools/gcra_metrics.py shard.log [label=HH:MM:SS-HH:MM:SS ...]
Each window is one clumsy profile; with no windows the whole log is one window.

FIRE offset = ms after the weapon was due (negative = early). "swap" = the
player's previous FIRE used a different weapon. HIT lateness = ms after the
weapon's flight time (m_iDurationTime) that the HIT arrived.
"""
import os, re, sys, collections

os.system("")  # enables ANSI colors in the Windows console
sys.stdout.reconfigure(encoding="utf-8")

FIRE = re.compile(r"\[gcra\] pc=(\d+) kind=(\w+) weapon=(\d+) offset=(-?\d+|first) cooldown=(\d+) allowance=(\d+)(?: hit_allowance=(\d+))? verdict=(\w+)")
HIT = re.compile(r"\[projectile-timing\] .* (rocket|grenade) (?:projectile \d+ hit|hit for expired projectile \d+ \() ?(\d+) ms after fire,? \(?limit (\d+)")
TOO_FAST = re.compile(r"too fast \(")

C = dict(reset="\033[0m", dim="\033[90m", gray="\033[37m", white="\033[97m", cyan="\033[96m",
         dcyan="\033[36m", green="\033[92m", yellow="\033[93m", red="\033[91m")

def c(color, text):
    return f"{C[color]}{text}{C['reset']}"

def visible_len(s):
    return len(re.sub(r"\033\[\d+m", "", s))

def pad(s, w, right=False):
    gap = " " * max(0, w - visible_len(s))
    return gap + s if right else s + gap

def pct(xs, q):
    return xs[min(len(xs) - 1, int(q * len(xs)))]

def banner(text):
    w = len(text) + 4
    print()
    print(c("dcyan", "╔" + "═" * w + "╗"))
    print(c("dcyan", "║  ") + c("cyan", text) + c("dcyan", "  ║"))
    print(c("dcyan", "╚" + "═" * w + "╝"))

def table(caption, headers, rows, right_from=1):
    widths = [max(visible_len(str(h)), *(visible_len(str(r[i])) for r in rows)) for i, h in enumerate(headers)]
    line = lambda l, m, r: c("dim", l + m.join("─" * (w + 2) for w in widths) + r)
    print()
    print(" " + c("white", caption))
    print(line("┌", "┬", "┐"))
    print(c("dim", "│") + c("dim", "│").join(" " + c("gray", pad(h, w, i >= right_from)) + " " for i, (h, w) in enumerate(zip(headers, widths))) + c("dim", "│"))
    print(line("├", "┼", "┤"))
    for r in rows:
        print(c("dim", "│") + c("dim", "│").join(" " + pad(str(v), w, i >= right_from) + " " for i, (v, w) in enumerate(zip(r, widths))) + c("dim", "│"))
    print(line("└", "┴", "┘"))

def histogram(caption, values, lo, hi, step, marker, marker_label, bad_side):
    """bad_side: 'left' = values below marker are bad, 'right' = above marker are bad."""
    if not values:
        return
    bins = collections.Counter()
    for v in values:
        b = max(lo - step, min(hi, (v // step) * step))
        bins[b] += 1
    top = max(bins.values())
    print()
    print(" " + c("white", caption))
    rows = []
    for b in range(lo - step, hi + step, step):
        n = bins.get(b, 0)
        if b == lo - step:
            label = f"< {lo}"
        elif b == hi:
            label = f">= {hi}"
        else:
            label = f"{b}..{b + step - 1}"
        is_marker = b <= marker < b + step
        bad = (b + step <= marker) if bad_side == "left" else (b >= marker)
        bar = "█" * round(30 * n / top) if n else ""
        color = "red" if bad and n else ("yellow" if abs(b - marker) < step and n else "green")
        tick = c("yellow", " ◀ " + marker_label) if is_marker else ""
        line = f"   {c('gray', label.rjust(11))} {c('dim', '│')}{c(color, bar)} {c('gray', str(n) if n else '')}{tick}"
        rows.append((n == 0 and not is_marker, line))
    # collapse runs of 3+ empty rows into one gap line
    i = 0
    while i < len(rows):
        j = i
        while j < len(rows) and rows[j][0]:
            j += 1
        if j - i >= 3:
            print(f"   {'':>11} {c('dim', '┊')}{c('dim', f' ({j - i} empty)')}")
            i = j
        elif j > i:
            for k in range(i, j):
                print(rows[k][1])
            i = j
        else:
            print(rows[i][1])
            i += 1

def verdict(ok_color, text):
    print("   " + c(ok_color, "●") + " " + text)

def analyze(lines, label, a, b):
    last_weapon = {}
    fires = collections.defaultdict(list)   # (kind, swap) -> [offset]
    rejects = collections.Counter()          # (kind, swap) -> n
    firsts = 0
    allowance = None
    hit_allowance = None
    hits = collections.defaultdict(list)     # kind -> [(ms, limit)]
    too_fast = 0
    for l in lines:
        t = l[12:20]
        if not (a <= t <= b):
            continue
        m = FIRE.search(l)
        if m:
            pc, kind, weapon, off, _cd, allow, hit_allow, v = m.groups()
            allowance = int(allow)
            hit_allowance = int(hit_allow) if hit_allow else allowance
            swap = pc in last_weapon and last_weapon[pc] != weapon
            last_weapon[pc] = weapon
            if off == "first":
                firsts += 1
                continue
            fires[(kind, swap)].append(int(off))
            if v == "reject":
                rejects[(kind, swap)] += 1
            continue
        m = HIT.search(l)
        if m:
            hits[m.group(1)].append((int(m.group(2)), int(m.group(3))))
            continue
        if TOO_FAST.search(l):
            too_fast += 1

    allowance = allowance or 150
    hit_allowance = hit_allowance or allowance
    n_fire = sum(len(v) for v in fires.values()) + firsts
    n_hit = sum(len(v) for v in hits.values())
    banner(f"GCRA benchmark: {label}   {a} - {b}")
    print(c("gray", f"  {n_fire} FIREs ({firsts} first-shot-of-weapon, not rated), {n_hit} HITs, FIRE allowance {allowance} ms, HIT allowance {hit_allowance} ms"))
    n_rej = sum(rejects.values())
    if n_rej != too_fast:
        print(c("yellow", f"  ! check: {n_rej} [gcra] rejects but {too_fast} 'too fast' warnings (slot-full drops or log mismatch)"))
    else:
        print(c("dim", f"  check: {n_rej} rejects = {too_fast} 'too fast' warnings"))

    # FIRE table
    rows = []
    summary = {}
    for (kind, swap), offs in sorted(fires.items()):
        offs.sort()
        rej = rejects[(kind, swap)]
        rate = 100 * rej / len(offs)
        headroom = allowance + offs[0]
        rate_s = c("green" if rej == 0 else ("yellow" if swap else "red"), f"{rej} ({rate:.1f}%)")
        head_s = c("green" if headroom >= 50 else ("yellow" if headroom >= 0 else "red"), f"{headroom}")
        rows.append([f"{kind} {'after swap' if swap else 'same weapon'}", len(offs), rate_s,
                     offs[0], pct(offs, .05), pct(offs, .5), head_s, max(0, -offs[0])])
        summary[(kind, swap)] = (len(offs), rej, offs[0])
    if rows:
        table("FIRE rate limit  (false alarms = legit shots rejected; want 0 on 'same weapon')",
              ["shots", "n", "rejected", "earliest", "p5", "p50", "headroom", "allowance for 0"], rows)
    same = sorted(o for (k, s), v in fires.items() if not s for o in v)
    histogram("FIRE offset vs due time, same weapon (ms, negative = early)", same,
              -allowance - 50, 300, 25, -allowance, f"allowance -{allowance}", "left")

    # HIT table
    rows = []
    late = []
    hit_summary = {}
    for kind, v in sorted(hits.items()):
        limit = min(y for _, y in v)
        duration = limit - hit_allowance
        ms = sorted(x for x, _ in v)
        expired = sum(1 for x, y in v if x >= y)
        full = [x for x in ms if x >= duration - 300]  # hits that flew (about) the whole way
        late += [x - duration for x in full]
        need = max(0, ms[-1] - duration + 1)
        margin = limit - ms[-1]
        exp_s = c("green" if expired == 0 else "red", f"{expired} ({100 * expired / len(v):.0f}%)")
        mar_s = c("green" if margin >= 50 else ("yellow" if margin >= 0 else "red"), f"{margin}")
        rows.append([kind, len(v), exp_s, len(full), duration, pct(ms, .5), pct(ms, .99), ms[-1], limit, mar_s, need])
        hit_summary[kind] = (len(v), expired, need)
    if rows:
        table("HIT expiry  (expired = real hits thrown away; want 0)",
              ["weapon", "n", "expired", "full flight", "flight", "p50", "p99", "max", "limit", "margin", "allowance for 0"], rows)
    histogram("HIT lateness after flight time, full-flight hits (ms)", sorted(late),
              -100, max(400, hit_allowance), 25, hit_allowance, f"allowance +{hit_allowance}", "right")

    # Verdicts
    print()
    print(" " + c("white", "Verdict"))
    for (kind, swap), (n, rej, earliest) in sorted(summary.items()):
        if swap:
            continue
        if rej == 0:
            verdict("green", f"FIRE {kind}: 0/{n} false alarms, earliest {earliest} ms ({allowance + earliest} ms headroom)")
        else:
            verdict("red", f"FIRE {kind}: {rej}/{n} false alarms; needs allowance >= {-earliest} ms")
    for (kind, swap), (n, rej, earliest) in sorted(summary.items()):
        if swap and rej:
            verdict("yellow", f"FIRE {kind} after swap: {rej}/{n} rejected (per-weapon / client-reset issue, not jitter)")
    for kind, (n, expired, need) in sorted(hit_summary.items()):
        if expired == 0:
            verdict("green", f"HIT {kind}: 0/{n} expired")
        else:
            verdict("red", f"HIT {kind}: {expired}/{n} real hits expired; HIT allowance needs >= {need} ms (now {hit_allowance})")
    return label, summary, hit_summary

def main():
    if len(sys.argv) < 2:
        print(__doc__)
        return
    lines = open(sys.argv[1], encoding="utf-8", errors="replace").read().splitlines()
    windows = []
    for arg in sys.argv[2:]:
        label, span = arg.split("=")
        a, b = span.split("-")
        windows.append((label, a, b))
    if not windows:
        windows = [("whole log", "00:00:00", "23:59:59")]

    results = [analyze(lines, *w) for w in windows]

    if len(results) > 1:
        rows = []
        for label, summary, hit_summary in results:
            fa = lambda k: summary.get((k, False))
            hs = lambda k: hit_summary.get(k)
            cell_f = lambda s: "-" if not s else c("green" if s[1] == 0 else "red", f"{s[1]}/{s[0]}  ({s[2]} ms)")
            cell_h = lambda s: "-" if not s else c("green" if s[1] == 0 else "red", f"{s[1]}/{s[0]}  (need {s[2]})")
            rows.append([label, cell_f(fa("rocket")), cell_f(fa("grenade")), cell_h(hs("rocket")), cell_h(hs("grenade"))])
        banner("Comparison across windows")
        table("false alarms (earliest shot) / expired hits (HIT allowance needed)",
              ["window", "FIRE rocket", "FIRE grenade", "HIT rocket", "HIT grenade"], rows, right_from=99)
    print()

main()
