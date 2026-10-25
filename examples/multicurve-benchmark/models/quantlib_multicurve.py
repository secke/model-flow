"""Pricer candidat QuantLib multi-courbes : €STR (OISRateHelper) pour l'actualisation, Euribor 6M
(DepositRateHelper + SwapRateHelper actualisés €STR) pour la projection. Conventions de marché EUR :
TARGET, spot T+2, Modified Following, fixe annuel 30/360, variable semestriel Act/360.
Protocole ModelFlow : JSON sur stdin -> JSON sur stdout."""
import json, sys

import QuantLib as ql

req = json.load(sys.stdin)
trades = req["inputs"]["trades"]["trades"]
market = req["inputs"]["market"]

y, m, d = map(int, market["as_of"].split("-"))
TODAY = ql.Date(d, m, y)
ql.Settings.instance().evaluationDate = TODAY
q = lambda r: ql.QuoteHandle(ql.SimpleQuote(r))

ois_spec = market["curves"]["EUR-ESTR"]
estr_h = ql.RelinkableYieldTermStructureHandle()
estr = ql.Estr(estr_h)
ois_helpers = [ql.OISRateHelper(2, ql.Period(i["tenor"]), q(i["rate"]), estr) for i in ois_spec["instruments"]]
ois_curve = ql.PiecewiseLogLinearDiscount(TODAY, ois_helpers, ql.Actual365Fixed())
ois_curve.enableExtrapolation()
estr_h.linkTo(ois_curve)

ibor_spec = market["curves"]["EUR-6M"]
fixed_dc = ql.Thirty360(ql.Thirty360.BondBasis)
helpers = []
for i in ibor_spec["instruments"]:
    if i["type"] == "deposit":
        helpers.append(ql.DepositRateHelper(q(i["rate"]), ql.Euribor6M()))
    else:
        helpers.append(
            ql.SwapRateHelper(q(i["rate"]), ql.Period(i["tenor"]), ql.TARGET(), ql.Annual, ql.ModifiedFollowing,
                              fixed_dc, ql.Euribor6M(), ql.QuoteHandle(), ql.Period(0, ql.Days), estr_h)
        )
proj = ql.PiecewiseLogLinearDiscount(TODAY, helpers, ql.Actual365Fixed())
proj.enableExtrapolation()
proj_h = ql.YieldTermStructureHandle(proj)
euribor6m = ql.Euribor6M(proj_h)
engine = ql.DiscountingSwapEngine(estr_h)


def swap(t):
    s = ql.MakeVanillaSwap(
        ql.Period(t["tenor"]), euribor6m, t["fixed_rate"], ql.Period(0, ql.Days),
        nominal=t["notional"], receiveFixed=not t.get("pay_fixed", True),
        fixedLegTenor=ql.Period(1, ql.Years), fixedLegDayCount=fixed_dc,
        fixedLegConvention=ql.ModifiedFollowing, fixedLegTerminationDateConvention=ql.ModifiedFollowing,
        pricingEngine=engine,
    )
    return s.NPV()


json.dump(
    {"library": f"QuantLib {ql.__version__}", "valuations": [{"id": t["id"], "price": swap(t)} for t in trades]},
    sys.stdout,
)
