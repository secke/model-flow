"""Writes the ORE inputs of the experiment: a portfolio over the five asset classes, three netting sets.

    python3 portfolio.py            # writes ore/Input/*.xml
"""
import pathlib

HERE = pathlib.Path(__file__).resolve().parent
OUT = HERE / "ore/Input"


def envelope(cpty):
    return f"<Envelope><CounterParty>{cpty}</CounterParty><NettingSetId>{cpty}</NettingSetId><AdditionalFields/></Envelope>"


def schedule(start, end, tenor, calendar, convention="MF"):
    return (f"<ScheduleData><Rules><StartDate>{start}</StartDate><EndDate>{end}</EndDate><Tenor>{tenor}</Tenor>"
            f"<Calendar>{calendar}</Calendar><Convention>{convention}</Convention><TermConvention>{convention}</TermConvention>"
            "<Rule>Forward</Rule></Rules></ScheduleData>")


IBOR = {"EUR": ("EUR-EURIBOR-6M", "6M", "TARGET"), "USD": ("USD-LIBOR-3M", "3M", "US"), "GBP": ("GBP-LIBOR-6M", "6M", "UK")}


def swap_legs(ccy, notional, rate, start, end, pay_fixed):
    index, tenor, cal = IBOR[ccy]
    fixed = (f"<LegData><LegType>Fixed</LegType><Payer>{str(pay_fixed).lower()}</Payer><Currency>{ccy}</Currency>"
             f"<Notionals><Notional>{notional}</Notional></Notionals><DayCounter>30/360</DayCounter><PaymentConvention>F</PaymentConvention>"
             f"<FixedLegData><Rates><Rate>{rate}</Rate></Rates></FixedLegData>{schedule(start, end, '1Y', cal, 'F')}</LegData>")
    floating = (f"<LegData><LegType>Floating</LegType><Payer>{str(not pay_fixed).lower()}</Payer><Currency>{ccy}</Currency>"
                f"<Notionals><Notional>{notional}</Notional></Notionals><DayCounter>A360</DayCounter><PaymentConvention>MF</PaymentConvention>"
                f"<FloatingLegData><Index>{index}</Index><Spreads><Spread>0</Spread></Spreads><IsInArrears>false</IsInArrears>"
                f"<FixingDays>2</FixingDays></FloatingLegData>{schedule(start, end, tenor, cal)}</LegData>")
    return fixed + floating


def option(long, kind, expiry, settlement="Cash"):
    return (f"<OptionData><LongShort>{'Long' if long else 'Short'}</LongShort><OptionType>{kind}</OptionType><Style>European</Style>"
            f"<Settlement>{settlement}</Settlement><PayOffAtExpiry>false</PayOffAtExpiry>"
            f"<ExerciseDates><ExerciseDate>{expiry}</ExerciseDate></ExerciseDates></OptionData>")


def trade(tid, ttype, cpty, body):
    return f'<Trade id="{tid}"><TradeType>{ttype}</TradeType>{envelope(cpty)}{body}</Trade>'


def swap(tid, cpty, ccy, notional, rate, start, end, pay_fixed):
    return trade(tid, "Swap", cpty, f"<SwapData>{swap_legs(ccy, notional, rate, start, end, pay_fixed)}</SwapData>")


def swaption(tid, cpty, ccy, notional, rate, expiry, start, end, long, pay_fixed):
    return trade(tid, "Swaption", cpty, f"<SwaptionData>{option(long, 'Call', expiry, 'Physical')}"
                 f"{swap_legs(ccy, notional, rate, start, end, pay_fixed)}</SwaptionData>")


def fx_forward(tid, cpty, date, bought, bamount, sold, samount):
    return trade(tid, "FxForward", cpty, f"<FxForwardData><ValueDate>{date}</ValueDate><BoughtCurrency>{bought}</BoughtCurrency>"
                 f"<BoughtAmount>{bamount}</BoughtAmount><SoldCurrency>{sold}</SoldCurrency><SoldAmount>{samount}</SoldAmount></FxForwardData>")


def fx_option(tid, cpty, expiry, long, kind, bought, bamount, sold, samount):
    return trade(tid, "FxOption", cpty, f"<FxOptionData>{option(long, kind, expiry)}<BoughtCurrency>{bought}</BoughtCurrency>"
                 f"<BoughtAmount>{bamount}</BoughtAmount><SoldCurrency>{sold}</SoldCurrency><SoldAmount>{samount}</SoldAmount></FxOptionData>")


def equity_option(tid, cpty, name, ccy, expiry, long, kind, strike, quantity):
    return trade(tid, "EquityOption", cpty, f"<EquityOptionData>{option(long, kind, expiry)}<Name>{name}</Name><Currency>{ccy}</Currency>"
                 f"<Strike>{strike}</Strike><Quantity>{quantity}</Quantity></EquityOptionData>")


def equity_forward(tid, cpty, name, ccy, maturity, long, strike, quantity):
    return trade(tid, "EquityForward", cpty, f"<EquityForwardData><LongShort>{'Long' if long else 'Short'}</LongShort>"
                 f"<Maturity>{maturity}</Maturity><Name>{name}</Name><Currency>{ccy}</Currency><Strike>{strike}</Strike>"
                 f"<Quantity>{quantity}</Quantity></EquityForwardData>")


def commodity_forward(tid, cpty, name, maturity, long, strike, quantity):
    return trade(tid, "CommodityForward", cpty, f"<CommodityForwardData><Position>{'Long' if long else 'Short'}</Position>"
                 f"<Maturity>{maturity}</Maturity><Name>{name}</Name><Currency>USD</Currency><Strike>{strike}</Strike>"
                 f"<Quantity>{quantity}</Quantity></CommodityForwardData>")


def commodity_option(tid, cpty, name, expiry, long, kind, strike, quantity):
    return trade(tid, "CommodityOption", cpty, f"<CommodityOptionData>{option(long, kind, expiry)}<Name>{name}</Name>"
                 f"<Currency>USD</Currency><Strike>{strike}</Strike><Quantity>{quantity}</Quantity></CommodityOptionData>")


# As of 2016-02-05. Netting set A: unmargined, four asset classes (ORE 1.8.17 has no SA-CCR for credit default swaps).
# B: the same trades under a CSA. C: rates only, margined, with excess collateral (multiplier below 1). D: one FX forward
# under a CSA with a large minimum transfer amount, where the margined EAD exceeds the unmargined one.
def book(cpty, suffix=""):
    s = suffix
    return [
        swap(f"IR_EUR_9M{s}", cpty, "EUR", 20e6, 0.0, "2016-02-09", "2016-11-09", True),
        swap(f"IR_EUR_3Y{s}", cpty, "EUR", 15e6, 0.002, "2016-02-09", "2019-02-11", True),
        swap(f"IR_EUR_10Y{s}", cpty, "EUR", 10e6, 0.01, "2016-02-09", "2026-02-09", False),
        swap(f"IR_EUR_FWD{s}", cpty, "EUR", 8e6, 0.012, "2018-02-09", "2028-02-09", True),
        swap(f"IR_USD_5Y{s}", cpty, "USD", 12e6, 0.015, "2016-02-09", "2021-02-09", True),
        swap(f"IR_GBP_7Y{s}", cpty, "GBP", 6e6, 0.016, "2016-02-09", "2023-02-09", False),
        swaption(f"IR_SWPT_EUR{s}", cpty, "EUR", 10e6, 0.008, "2017-02-07", "2017-02-09", "2022-02-09", True, True),
        swaption(f"IR_SWPT_USD{s}", cpty, "USD", 10e6, 0.02, "2018-02-07", "2018-02-09", "2028-02-09", False, False),
        fx_forward(f"FX_EURUSD_1Y{s}", cpty, "2017-02-07", "USD", 11.5e6, "EUR", 10e6),
        fx_forward(f"FX_EURGBP_6M{s}", cpty, "2016-08-08", "EUR", 5e6, "GBP", 4.1e6),
        fx_forward(f"FX_GBPUSD_2Y{s}", cpty, "2018-02-07", "GBP", 3e6, "USD", 4.4e6),
        fx_option(f"FX_OPT_EURUSD{s}", cpty, "2017-02-07", True, "Call", "EUR", 8e6, "USD", 9.2e6),
        equity_option(f"EQ_OPT_SP5_C{s}", cpty, "SP5", "USD", "2017-02-06", True, "Call", 2200, 3000),
        equity_option(f"EQ_OPT_SP5_P{s}", cpty, "SP5", "USD", "2016-08-05", False, "Put", 2000, 2000),
        equity_forward(f"EQ_FWD_LHA{s}", cpty, "Lufthansa", "EUR", "2016-11-07", True, 13, 400000),
        commodity_forward(f"CO_FWD_WTI{s}", cpty, "COMDTY_WTI_USD", "2017-02-06", True, 38, 150000),
        commodity_forward(f"CO_FWD_GOLD{s}", cpty, "COMDTY_GOLD_USD", "2018-02-05", False, 1170, 4000),
        commodity_option(f"CO_OPT_WTI{s}", cpty, "COMDTY_WTI_USD", "2016-12-05", True, "Call", 40, 100000),
    ]


trades = book("CPTY_A") + book("CPTY_B", "_B") + [
    swap("IR_EUR_5Y_C", "CPTY_C", "EUR", 25e6, 0.003, "2016-02-09", "2021-02-09", False),
    swap("IR_EUR_2Y_C", "CPTY_C", "EUR", 25e6, 0.0, "2016-02-09", "2018-02-09", True),
    fx_forward("FX_EURUSD_6M_D", "CPTY_D", "2016-08-08", "USD", 5.7e6, "EUR", 5e6),
]


def csa(nid, active, threshold, mta, mpor, ia=0):
    return (f"<NettingSet><NettingSetId>{nid}</NettingSetId><ActiveCSAFlag>{str(active).lower()}</ActiveCSAFlag><CSADetails>"
            f"<Bilateral>Bilateral</Bilateral><CSACurrency>EUR</CSACurrency><Index>EUR-EONIA</Index>"
            f"<ThresholdPay>{threshold}</ThresholdPay><ThresholdReceive>{threshold}</ThresholdReceive>"
            f"<MinimumTransferAmountPay>{mta}</MinimumTransferAmountPay><MinimumTransferAmountReceive>{mta}</MinimumTransferAmountReceive>"
            f"<IndependentAmount><IndependentAmountHeld>{ia}</IndependentAmountHeld><IndependentAmountType>FIXED</IndependentAmountType></IndependentAmount>"
            f"<MarginingFrequency><CallFrequency>1D</CallFrequency><PostFrequency>1D</PostFrequency></MarginingFrequency>"
            f"<MarginPeriodOfRisk>{mpor}</MarginPeriodOfRisk><CollateralCompoundingSpreadReceive>0.00</CollateralCompoundingSpreadReceive>"
            f"<CollateralCompoundingSpreadPay>0.00</CollateralCompoundingSpreadPay>"
            f"<EligibleCollaterals><Currencies><Currency>EUR</Currency></Currencies></EligibleCollaterals></CSADetails></NettingSet>")


def balance(nid, im, vm):
    return (f"<CollateralBalance><NettingSetId>{nid}</NettingSetId><Currency>EUR</Currency>"
            f"<InitialMargin>{im}</InitialMargin><VariationMargin>{vm}</VariationMargin></CollateralBalance>")


def counterparty(cid, quality):
    return (f"<Counterparty><CounterpartyId>{cid}</CounterpartyId><CreditQuality>{quality}</CreditQuality>"
            "<BaCvaRiskWeight>0.05</BaCvaRiskWeight><SaCcrRiskWeight>1</SaCcrRiskWeight><SaCvaRiskBucket>2</SaCvaRiskBucket></Counterparty>")


OUT.mkdir(parents=True, exist_ok=True)
(OUT / "portfolio.xml").write_text('<?xml version="1.0"?>\n<Portfolio>\n' + "\n".join(trades) + "\n</Portfolio>\n")
(OUT / "netting.xml").write_text('<?xml version="1.0"?>\n<NettingSetDefinitions>\n' + "\n".join([
    csa("CPTY_A", False, 0, 0, "0W"),
    csa("CPTY_B", True, 1000000, 250000, "2W"),
    csa("CPTY_C", True, 0, 0, "2W"),
    csa("CPTY_D", True, 0, 3000000, "2W"),
]) + "\n</NettingSetDefinitions>\n")
(OUT / "collateralbalances.xml").write_text('<?xml version="1.0"?>\n<CollateralBalances>\n' + "\n".join([
    balance("CPTY_B", 500000, 300000),
    balance("CPTY_C", 2000000, 0),
]) + "\n</CollateralBalances>\n")
(OUT / "counterparty.xml").write_text('<?xml version="1.0"?>\n<CounterpartyInformation><Counterparties>\n' + "\n".join([
    counterparty("CPTY_A", "IG"), counterparty("CPTY_B", "IG"), counterparty("CPTY_C", "HY"), counterparty("CPTY_D", "IG"),
]) + "\n</Counterparties></CounterpartyInformation>\n")
# ORE's SA-CCR maps commodity and equity names to SIMM qualifiers, and commodity qualifiers to SIMM buckets (hedging sets).
(OUT / "simmnames.xml").write_text("""<?xml version="1.0"?>
<SIMMNameMappings>
  <Mapping><Name>COMDTY_WTI_USD</Name><Qualifier>Crude oil Americas</Qualifier></Mapping>
  <Mapping><Name>COMDTY_GOLD_USD</Name><Qualifier>Precious Metals Gold</Qualifier></Mapping>
  <Mapping><Name>SP5</Name><Qualifier>SP5</Qualifier></Mapping>
  <Mapping><Name>Lufthansa</Name><Qualifier>Lufthansa</Qualifier></Mapping>
</SIMMNameMappings>
""")
(OUT / "simmbuckets.xml").write_text("""<?xml version="1.0"?>
<SIMMBucketMappings>
  <Risk_Commodity>
    <Mapping><Qualifier>Crude oil Americas</Qualifier><Bucket>2</Bucket></Mapping>
    <Mapping><Qualifier>Precious Metals Gold</Qualifier><Bucket>12</Bucket></Mapping>
  </Risk_Commodity>
</SIMMBucketMappings>
""")
(OUT.parent / "ore.xml").write_text("""<?xml version="1.0"?>
<ORE>
  <Setup>
    <Parameter name="asofDate">2016-02-05</Parameter>
    <Parameter name="inputPath">Input</Parameter>
    <Parameter name="outputPath">Output</Parameter>
    <Parameter name="logFile">log.txt</Parameter>
    <Parameter name="logMask">31</Parameter>
    <Parameter name="marketDataFile">../../data/market/market_20160205_flat.txt</Parameter>
    <Parameter name="fixingDataFile">../../data/market/fixings_20160205.txt</Parameter>
    <Parameter name="implyTodaysFixings">N</Parameter>
    <Parameter name="curveConfigFile">../../data/market/curveconfig.xml</Parameter>
    <Parameter name="conventionsFile">../../data/market/conventions.xml</Parameter>
    <Parameter name="marketConfigFile">../../data/market/todaysmarket.xml</Parameter>
    <Parameter name="pricingEnginesFile">../../data/market/pricingengine.xml</Parameter>
    <Parameter name="portfolioFile">portfolio.xml</Parameter>
    <Parameter name="counterpartyFile">counterparty.xml</Parameter>
    <Parameter name="observationModel">Disable</Parameter>
    <Parameter name="nThreads">1</Parameter>
  </Setup>
  <Markets>
    <Parameter name="lgmcalibration">collateral_inccy</Parameter>
    <Parameter name="fxcalibration">xois_eur</Parameter>
    <Parameter name="pricing">xois_eur</Parameter>
    <Parameter name="simulation">xois_eur</Parameter>
    <Parameter name="sensitivity">xois_eur</Parameter>
  </Markets>
  <Analytics>
    <Analytic type="npv">
      <Parameter name="active">Y</Parameter>
      <Parameter name="baseCurrency">EUR</Parameter>
      <Parameter name="outputFileName">npv.csv</Parameter>
    </Analytic>
    <Analytic type="additionalResults">
      <Parameter name="active">Y</Parameter>
      <Parameter name="outputFileName">additional_results.csv</Parameter>
    </Analytic>
    <Analytic type="saccr">
      <Parameter name="active">Y</Parameter>
      <Parameter name="simmVersion">2.1</Parameter>
      <Parameter name="nameMappingInputFile">simmnames.xml</Parameter>
      <Parameter name="bucketMappingInputFile">simmbuckets.xml</Parameter>
      <Parameter name="csaFile">netting.xml</Parameter>
      <Parameter name="collateralBalancesFile">collateralbalances.xml</Parameter>
    </Analytic>
  </Analytics>
</ORE>
""")
print(f"{len(trades)} trades written to {OUT}")
