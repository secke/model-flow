"""SA-CCR of the ORE portfolio by quantparity, under Basel CRE52 and under ORE's conventions, against ORE 1.8.17.

    python3 compare.py      # after ORE ran on ore/ore.xml; writes netting_sets.json, results/summary.md, results/summary.json

The trades are read from ORE's own input (ore/Input). From ORE's output, only what a pricer gives is taken: market values,
forward prices and, for ORE's conventions, the payment dates. Everything SA-CCR computes is computed again.
"""
import copy
import csv
import json
import math
import pathlib
import xml.etree.ElementTree as ET
from collections import defaultdict
from datetime import date

import quantparity as qp

HERE = pathlib.Path(__file__).resolve().parent
INP, OUT = HERE / "ore/Input", HERE / "ore/Output"
TODAY = date(2016, 2, 5)
ALPHA = 1.4


def ymd(s):
    return date.fromisoformat(s if "-" in s else f"{s[:4]}-{s[4:6]}-{s[6:]}")


def act_act(d):
    """Actual/Actual (ISDA) year fraction from TODAY, ORE's day count for SA-CCR."""
    d = ymd(d) if isinstance(d, str) else d
    if d <= TODAY:
        return 0.0
    t, y = 0.0, TODAY.year
    start = TODAY
    while start < d:
        end = min(d, date(y + 1, 1, 1))
        t += (end - start).days / (366 if y % 4 == 0 and (y % 100 or y % 400 == 0) else 365)
        start, y = end, y + 1
    return t


def rows(name):
    with open(OUT / name) as f:
        return list(csv.DictReader(f, delimiter=","))


def keyed(name):
    return {r["#TradeId"]: r for r in rows(name) if r.get("#TradeId")}


# Market inputs ORE's pricer gives: FX spots (EUR per unit of currency), market values in EUR, forward prices.
fx = {"EUR": 1.0}
for line in open(HERE / "data/market/market_20160205_flat.txt"):
    parts = line.split()
    if len(parts) == 3 and parts[1] in ("FX/RATE/EUR/USD", "FX/RATE/EUR/GBP"):
        fx[parts[1][-3:]] = 1.0 / float(parts[2])
npv = {k: float(r["NPV(Base)"]) for k, r in keyed("npv.csv").items()}
extra = defaultdict(dict)
for r in rows("additional_results.csv"):
    if r["ResultType"] == "double":
        extra[r["#TradeId"]][r["ResultId"]] = float(r["ResultValue"])
payments = defaultdict(list)
for r in rows("cashflow.csv"):
    if r["PayDate"] != "#N/A":
        payments[r["TradeId"]].append(ymd(r["PayDate"]))
# Current price of one unit (CRE52): the equity spot; for crude oil and gold the nearest quoted price, as ORE's own
# commodity option engine does (the market has no commodity spot quote).
SPOT = {"SP5": 2147.56 * fx["USD"], "Lufthansa": 12.75, "COMDTY_WTI_USD": 30.89 * fx["USD"], "COMDTY_GOLD_USD": 1157.8 * fx["USD"]}
COMMODITY = {"COMDTY_WTI_USD": ("Crude oil", "oil_gas"), "COMDTY_GOLD_USD": ("Gold", "metals")}
UNITS = {}  # trade id: (quantity, EUR per unit of the price currency)


def text(node, path):
    return node.findtext(path)


def fx_notional(bought, bamount, sold, samount):
    """Foreign leg in EUR; with two foreign legs, the larger (CRE52). Long the pair = bought its first currency."""
    legs = {bought: bamount * fx[bought], sold: samount * fx[sold]}
    notional = max(v for c, v in legs.items() if c != "EUR")
    pair = "".join(sorted([bought, sold]))
    return pair, notional, bought == pair[:3]


def basel_trade(t):
    tid, ttype = t.get("id"), text(t, "TradeType")
    base = {"id": tid, "market_value": npv[tid]}
    if ttype in ("Swap", "Swaption"):
        data = t.find("SwapData") if ttype == "Swap" else t.find("SwaptionData")
        fixed = next(l for l in data.findall("LegData") if text(l, "LegType") == "Fixed")
        ccy = text(fixed, "Currency")
        start, end = text(fixed, "ScheduleData/Rules/StartDate"), text(fixed, "ScheduleData/Rules/EndDate")
        pay_fixed = text(fixed, "Payer") == "true"
        base.update(asset_class="ir", qualifier=ccy, notional=float(text(fixed, "Notionals/Notional")) * fx[ccy],
                    maturity=act_act(end), start=act_act(start), end=act_act(end))
        if ttype == "Swap":
            base["long"] = pay_fixed
        else:
            od = data.find("OptionData")
            # A payer swaption is a call on the swap rate, the primary risk factor (CRE52).
            base["option"] = {"kind": "call" if pay_fixed else "put", "bought": text(od, "LongShort") == "Long",
                              "price": extra[tid]["atmForward"], "strike": float(text(fixed, "FixedLegData/Rates/Rate")),
                              "exercise": act_act(text(od, "ExerciseDates/ExerciseDate"))}
    elif ttype in ("FxForward", "FxOption"):
        data = t.find(f"{ttype}Data")
        pair, notional, long = fx_notional(text(data, "BoughtCurrency"), float(text(data, "BoughtAmount")),
                                           text(data, "SoldCurrency"), float(text(data, "SoldAmount")))
        base.update(asset_class="fx", qualifier=pair, notional=notional)
        if ttype == "FxForward":
            base.update(long=long, maturity=act_act(text(data, "ValueDate")))
        else:
            od = data.find("OptionData")
            expiry = act_act(text(od, "ExerciseDates/ExerciseDate"))
            # Forward and strike in the second currency per unit of the first: the call buys the first currency.
            call = (text(od, "OptionType") == "Call") == long
            base.update(maturity=expiry, option={"kind": "call" if call else "put", "bought": text(od, "LongShort") == "Long",
                                                 "price": extra[tid]["forward"], "strike": extra[tid]["strike"], "exercise": expiry})
    elif ttype in ("EquityOption", "EquityForward", "CommodityOption", "CommodityForward"):
        data = t.find(f"{ttype}Data")
        name, quantity = text(data, "Name"), float(text(data, "Quantity"))
        equity = ttype.startswith("Equity")
        base["notional"] = SPOT[name] * quantity
        UNITS[tid] = (quantity, fx[text(data, "Currency")])
        if equity:
            base.update(asset_class="equity", qualifier=name, index=name == "SP5")
        else:
            kind, sector = COMMODITY[name]
            base.update(asset_class="commodity", qualifier=kind, sector=sector)
        if ttype.endswith("Forward"):
            side = text(data, "LongShort") if equity else text(data, "Position")
            base.update(long=side == "Long", maturity=act_act(text(data, "Maturity")))
        else:
            od = data.find("OptionData")
            expiry = act_act(text(od, "ExerciseDates/ExerciseDate"))
            base.update(maturity=expiry, option={"kind": text(od, "OptionType").lower(), "bought": text(od, "LongShort") == "Long",
                                                 "price": extra[tid]["forward"], "strike": float(text(data, "Strike")), "exercise": expiry})
    else:
        raise ValueError(f"{tid}: {ttype}")
    return base


# Netting sets and collateral, from ORE's netting set definitions and collateral balances.
portfolio = ET.parse(INP / "portfolio.xml").getroot()
balances = {text(b, "NettingSetId"): (float(text(b, "InitialMargin")), float(text(b, "VariationMargin")))
            for b in ET.parse(INP / "collateralbalances.xml").getroot()}
csas = {text(n, "NettingSetId"): n for n in ET.parse(INP / "netting.xml").getroot()}
reported = {r["#NettingSet"]: r for r in rows("saccr.csv") if r["AssetClass"] == "All" and r["#NettingSet"] != "All"}


def basel_netting_sets():
    sets = []
    for nid, n in csas.items():
        trades = [basel_trade(t) for t in portfolio if text(t, "Envelope/NettingSetId") == nid]
        ns = {"id": nid, "trades": trades, "collateral": 0.0}
        if text(n, "ActiveCSAFlag") == "true":
            im, vm = balances.get(nid, (0.0, 0.0))
            weeks = int(text(n, "CSADetails/MarginPeriodOfRisk").rstrip("W"))
            ns["collateral"] = im + vm
            ns["margin"] = {"threshold": float(text(n, "CSADetails/ThresholdReceive")),
                            "mta": float(text(n, "CSADetails/MinimumTransferAmountReceive")), "nica": im,
                            "mpor_days": 5.0 * weeks}
        sets.append(ns)
    return sets


# ORE's conventions, one at a time, each a change of quantparity's inputs (steps 1 to 6) or of its aggregation (7, 8).
def ore_dates(ns):
    for t in ns["trades"]:
        if t["asset_class"] == "ir":
            pays = payments[t["id"]]
            t["start"], t["end"] = act_act(min(pays)), act_act(max(pays))
            t["maturity"] = t["end"]


def ore_forward_prices(ns):
    for t in ns["trades"]:
        if t["id"] in UNITS:
            quantity, rate = UNITS[t["id"]]
            t["notional"] = quantity * rate * extra[t["id"]].get("forwardPrice", extra[t["id"]].get("forward"))


def ore_single_names(ns):
    for t in ns["trades"]:
        if t["asset_class"] == "equity":
            t["index"] = False


def ore_option_delta(ns):
    # phi(P, K, T, sigma) is called as phi(P, K, sigma, T): the expiry and the supervisory volatility trade places.
    vol = {"ir": 0.5, "fx": 0.15, "equity": 1.2, "commodity": 0.7}
    for t in ns["trades"]:
        if "option" in t:
            o = t["option"]
            o["exercise"], o["volatility"] = vol[t["asset_class"]], o["exercise"]


def ore_swaption_sign(ns):
    # A swaption paying fixed is taken as a put.
    for t in ns["trades"]:
        if t["asset_class"] == "ir" and "option" in t:
            t["option"]["kind"] = "put" if t["option"]["kind"] == "call" else "call"


def ore_mpor(ns):
    # MF = 1.5 sqrt(weeks / 52), against 1.5 sqrt(business days / 250).
    if "margin" in ns:
        ns["margin"]["mpor_days"] = 250 * (ns["margin"]["mpor_days"] / 5) / 52


STEPS = [
    ("Basel CRE52 (quantparity)", None),
    ("S and E: first and last payment dates, not the start and end of the swap", ore_dates),
    ("Equity and commodity notionals at the forward price, not the current price", ore_forward_prices),
    ("S&P 500 as a single name: factor 32%, correlation 0.5 (index: 20%, 0.8)", ore_single_names),
    ("Option delta with expiry and volatility swapped", ore_option_delta),
    ("Payer swaptions as puts", ore_swaption_sign),
    ("Margined maturity factor in weeks / 52, not business days / 250", ore_mpor),
]


def run(sets):
    return {ns["id"]: qp.saccr_exposure(ns, alpha=ALPHA) for ns in sets}


def no_cap(e):
    return ALPHA * (e["replacement_cost"] + e["pfe"])


def ore_aggregation(e, ns, usd_market_value):
    """ORE's IR add-on: 0.5% of the signed sum of δ·d·MF per currency, without maturity buckets or absolute value; the
    multiplier on that add-on; no cap. Other classes as computed (same formula in both). With `usd_market_value`, the
    netting set's market value is summed in USD and used as if in EUR, as ORE's calculator does with the CRIF PV records."""
    ir = defaultdict(float)
    for d in e["trades"]:
        if d["asset_class"] == "ir":
            ir[d["hedging_set"]] += 0.005 * d["effective_notional"]
    others = sum(c["add_on"] for c in e["asset_classes"] if c["asset_class"] != "ir")
    add_on = sum(ir.values()) + others
    v = e["market_value"] / fx["USD"] if usd_market_value else e["market_value"]
    vc = v - e["collateral"]
    m = ns.get("margin")
    rc = max(vc, m["threshold"] + m["mta"] - m["nica"], 0.0) if m else max(vc, 0.0)
    mult = min(1.0, 0.05 + 0.95 * math.exp(vc / (1.9 * add_on)))
    return {"ir": dict(ir), "add_on": add_on, "multiplier": mult, "pfe": mult * add_on, "rc": rc, "market_value": v,
            "ead": ALPHA * (rc + mult * add_on)}


sets = basel_netting_sets()
basel = run(sets)
bridge, current = [], copy.deepcopy(sets)
results = basel
for label, step in STEPS:
    if step:
        for ns in current:
            step(ns)
        results = run(current)
    bridge.append((label, {k: v["ead"] for k, v in results.items()}))
bridge.append(("Margined EAD not capped at the unmargined EAD", {k: no_cap(v) for k, v in results.items()}))
by_id = {ns["id"]: ns for ns in current}
ore_like = {k: ore_aggregation(v, by_id[k], False) for k, v in results.items()}
bridge.append(("IR add-on: signed sum per currency, no maturity buckets", {k: v["ead"] for k, v in ore_like.items()}))
ore_like = {k: ore_aggregation(v, by_id[k], True) for k, v in results.items()}
bridge.append(("Market value summed in USD, used as EUR", {k: v["ead"] for k, v in ore_like.items()}))
ore_ead = {k: float(r["EAD"]) for k, r in reported.items()}
bridge.append(("ORE 1.8.17", ore_ead))

# Per trade: quantparity with ORE's conventions against ORE's detail report (δ and MF to 4 decimals, d to 4).
detail = {r["#TradeId"]: r for r in rows("saccr_detail.csv")}
trades = []
for nid, e in results.items():
    for d in e["trades"]:
        o = detail[d["id"]]
        trades.append({"trade": d["id"], "netting_set": nid,
                       "delta": (d["delta"], float(o["delta"])),
                       "d": (d["adjusted_notional"], float(o["d"])),
                       "mf": (d["maturity_factor"], float(o["MF"])),
                       "basel_delta": next(b["delta"] for b in basel[nid]["trades"] if b["id"] == d["id"]),
                       "basel_d": next(b["adjusted_notional"] for b in basel[nid]["trades"] if b["id"] == d["id"])})
worst = {k: max(abs(t[k][0] - t[k][1]) / max(abs(t[k][1]), 1e-12) if k == "d" else abs(t[k][0] - t[k][1]) for t in trades)
         for k in ("delta", "d", "mf")}
hedging = []
for r in rows("saccr.csv"):
    if r["#NettingSet"] != "All" and r["AssetClass"] not in ("All",) and r["HedgingSet"] != "All":
        nid, cls, hs = r["#NettingSet"], r["AssetClass"], r["HedgingSet"]
        if cls == "IR":
            ours = ore_like[nid]["ir"][hs]
        else:
            name = {"FX": "fx", "Equity": "equity", "Commodity": "commodity"}[cls]
            c = next(c for c in results[nid]["asset_classes"] if c["asset_class"] == name)
            want = {"Energy": "energy", "Metal": "metals", "Equity": "all"}.get(hs, hs)
            ours = next(h["add_on"] for h in c["hedging_sets"] if h["name"].lower() == want.lower())
        hedging.append({"netting_set": nid, "asset_class": cls, "hedging_set": hs, "quantparity": ours, "ore": float(r["AddOn"])})

# Netting sets for the ModelFlow step: the Basel inputs, with ORE's figures as the reported exposures.
for ns in sets:
    r = reported[ns["id"]]
    ns.update(reported_ead=float(r["EAD"]), reported_rc=float(r["RC"]), reported_pfe=float(r["PFE"]))
(HERE / "netting_sets.json").write_text(json.dumps({"netting_sets": sets}, indent=1) + "\n")

ids = list(csas)
lines = ["## EAD, from Basel CRE52 to ORE 1.8.17, one convention at a time", "",
         "| Step | " + " | ".join(ids) + " |", "|---|" + "---:|" * len(ids)]
for label, ead in bridge:
    lines.append(f"| {label} | " + " | ".join(f"{ead[i]:,.0f}" for i in ids) + " |")
lines += ["", "## Basel exposure by netting set (quantparity)", "",
          "| Netting set | V | C | RC | Add-on | Multiplier | PFE | EAD | Unmargined EAD | ORE EAD |", "|---|" + "---:|" * 9]
for i in ids:
    e = basel[i]
    lines.append(f"| {i} | {e['market_value']:,.0f} | {e['collateral']:,.0f} | {e['replacement_cost']:,.0f} | {e['add_on']:,.0f} | "
                 f"{e['multiplier']:.4f} | {e['pfe']:,.0f} | {e['ead']:,.0f} | "
                 f"{e.get('unmargined_ead') or e['ead']:,.0f} | {ore_ead[i]:,.0f} |")
lines += ["", "## Hedging set add-ons: quantparity with ORE's conventions against ORE", "",
          "| Netting set | Class | Hedging set | quantparity | ORE | Difference |", "|---|---|---|---:|---:|---:|"]
for h in hedging:
    lines.append(f"| {h['netting_set']} | {h['asset_class']} | {h['hedging_set']} | {h['quantparity']:,.2f} | {h['ore']:,.2f} | "
                 f"{h['quantparity'] - h['ore']:.2e} |")
lines += ["", "## Trades, netting set CPTY_A: Basel against ORE", "",
          "| Trade | δ Basel | δ ORE | d Basel | d ORE | MF ORE |", "|---|---:|---:|---:|---:|---:|"]
for t in trades:
    if t["netting_set"] == "CPTY_A":
        lines.append(f"| {t['trade']} | {t['basel_delta']:.4f} | {t['delta'][1]:.4f} | {t['basel_d']:,.0f} | {t['d'][1]:,.0f} | {t['mf'][1]:.4f} |")
lines += ["", f"Per trade, quantparity with ORE's conventions against ORE's detail report ({len(trades)} trades): "
          f"largest difference {worst['delta']:.1e} in δ and {worst['mf']:.1e} in MF (ORE prints 4 decimals), "
          f"{worst['d']:.1e} relative in d.", ""]

(HERE / "results").mkdir(exist_ok=True)
(HERE / "results/summary.md").write_text("\n".join(lines))
(HERE / "results/summary.json").write_text(json.dumps({
    "bridge": [{"step": label, "ead": ead} for label, ead in bridge],
    "basel": {i: {k: basel[i][k] for k in ("market_value", "collateral", "replacement_cost", "add_on", "multiplier", "pfe", "ead",
                                           "capped")} | {"unmargined_ead": basel[i].get("unmargined_ead")} for i in ids},
    "ore": {i: {k: float(reported[i][k]) for k in ("NPV", "RC", "AddOn", "Multiplier", "PFE", "EAD")} for i in ids},
    "worst_trade_difference": worst, "hedging_sets": hedging,
}, indent=1) + "\n")
print("\n".join(lines))
