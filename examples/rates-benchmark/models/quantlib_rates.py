"""QuantLib candidate pricer for rates products (single-currency, single-curve).
Conventions aligned with the reference: null calendar, unadjusted dates and SimpleDayCounter
(year fraction = months / 12), PiecewiseLogLinearDiscount bootstrap.
ModelFlow protocol: JSON on stdin -> JSON on stdout."""
import json, sys

import QuantLib as ql

req = json.load(sys.stdin)
trades = req["inputs"]["trades"]["trades"]
market = req["inputs"]["market"]

TODAY = ql.Date(15, 1, 2026)
ql.Settings.instance().evaluationDate = TODAY
DC, CAL, U = ql.SimpleDayCounter(), ql.NullCalendar(), ql.Unadjusted
FREQ = {1: ql.Annual, 2: ql.Semiannual, 4: ql.Quarterly, 12: ql.Monthly}


def months(t):
    m = round(t * 12)
    if abs(m - t * 12) > 1e-9:
        raise ValueError(f"maturity {t} is not a whole number of months")
    return m


def build(spec):
    tenor = ql.Period(12 // spec.get("float_frequency", 2), ql.Months)
    handle = ql.RelinkableYieldTermStructureHandle()
    index = ql.IborIndex("MF", tenor, 0, ql.EURCurrency(), CAL, U, False, DC, handle)
    helpers = []
    for i in spec["instruments"]:
        q = ql.QuoteHandle(ql.SimpleQuote(i["rate"]))
        p = ql.Period(months(i["maturity"]), ql.Months)
        if i["type"] == "deposit":
            helpers.append(ql.DepositRateHelper(q, p, 0, CAL, U, False, DC))
        else:
            helpers.append(ql.SwapRateHelper(q, p, CAL, FREQ[i["fixed_frequency"]], U, DC, index))
    curve = ql.PiecewiseLogLinearDiscount(TODAY, helpers, DC)
    curve.enableExtrapolation()
    handle.linkTo(curve)
    return curve, handle, index


CURVES = {name: build(spec) for name, spec in market["curves"].items()}


def schedule(m, tenor):
    end = TODAY + ql.Period(m, ql.Months)
    return ql.Schedule(TODAY, end, tenor, CAL, U, U, ql.DateGeneration.Forward, False)


def swap(t):
    curve, handle, index = CURVES[t["curve"]]
    m = months(t["maturity"])
    s = ql.VanillaSwap(
        ql.VanillaSwap.Payer if t.get("pay_fixed", True) else ql.VanillaSwap.Receiver,
        t["notional"],
        schedule(m, ql.Period(FREQ[t["fixed_frequency"]])),
        t["fixed_rate"],
        DC,
        schedule(m, index.tenor()),
        index,
        0.0,
        DC,
    )
    s.setPricingEngine(ql.DiscountingSwapEngine(handle))
    return s.NPV()


def zcb(t):
    curve = CURVES[t["curve"]][0]
    return t.get("face", 100.0) * curve.discount(TODAY + ql.Period(months(t["maturity"]), ql.Months))


PRICERS = {"interest_rate_swap": swap, "zero_coupon_bond": zcb}
json.dump(
    {"library": f"QuantLib {ql.__version__}", "valuations": [{"id": t["id"], "price": PRICERS[t["product"]](t)} for t in trades]},
    sys.stdout,
)
