"""Pricer candidat basé sur QuantLib (bibliothèque de production à valider).
Protocole ModelFlow : JSON sur stdin -> JSON sur stdout."""
import json, math, sys

import QuantLib as ql

req = json.load(sys.stdin)
trades = req["inputs"]["trades"]["trades"]

TODAY = ql.Date(2, 1, 2026)
ql.Settings.instance().evaluationDate = TODAY
DC = ql.Actual365Fixed()
CAL = ql.NullCalendar()


def kind(t):
    return ql.Option.Call if t["option_type"] == "call" else ql.Option.Put


def european(t):
    T, q = t["maturity"], t.get("dividend", 0.0)
    fwd = t["spot"] * math.exp((t["rate"] - q) * T)
    return ql.blackFormula(kind(t), t["strike"], fwd, t["vol"] * math.sqrt(T), math.exp(-t["rate"] * T))


def american(t):
    days = round(t["maturity"] * 365)
    if abs(days - t["maturity"] * 365) > 1e-9:
        raise ValueError(f"{t['id']}: maturité non entière en jours (Act/365)")
    flat = lambda r: ql.YieldTermStructureHandle(ql.FlatForward(TODAY, r, DC))
    process = ql.BlackScholesMertonProcess(
        ql.QuoteHandle(ql.SimpleQuote(t["spot"])),
        flat(t.get("dividend", 0.0)),
        flat(t["rate"]),
        ql.BlackVolTermStructureHandle(ql.BlackConstantVol(TODAY, CAL, t["vol"], DC)),
    )
    option = ql.VanillaOption(ql.PlainVanillaPayoff(kind(t), t["strike"]), ql.AmericanExercise(TODAY, TODAY + days))
    option.setPricingEngine(ql.BinomialVanillaEngine(process, "crr", t.get("steps", 1000)))
    return option.NPV()


def black76(t):
    T = t["maturity"]
    return ql.blackFormula(kind(t), t["strike"], t["forward"], t["vol"] * math.sqrt(T), math.exp(-t["rate"] * T))


def bond(t):
    freq = {1: ql.Annual, 2: ql.Semiannual, 4: ql.Quarterly, 12: ql.Monthly}[t["frequency"]]
    maturity = TODAY + ql.Period(int(round(t["maturity"] * 12)), ql.Months)
    schedule = ql.Schedule(TODAY, maturity, ql.Period(freq), CAL, ql.Unadjusted, ql.Unadjusted, ql.DateGeneration.Backward, False)
    dc = ql.Thirty360(ql.Thirty360.BondBasis)
    b = ql.FixedRateBond(0, t.get("face", 100.0), schedule, [t["coupon_rate"]], dc)
    return b.cleanPrice(t["yield_rate"], dc, ql.Compounded, freq) * t.get("face", 100.0) / 100.0


PRICERS = {"european_option": european, "american_option": american, "black76": black76, "fixed_rate_bond": bond}

json.dump(
    {"library": f"QuantLib {ql.__version__}", "valuations": [{"id": t["id"], "price": PRICERS[t["product"]](t)} for t in trades]},
    sys.stdout,
)
