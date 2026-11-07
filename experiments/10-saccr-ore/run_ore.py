"""Runs ORE's SA-CCR analytic on ore/ore.xml (pip install open-source-risk-engine==1.8.17.0).

    python3 run_ore.py      # writes ore/Output
"""
import os
import pathlib

import ORE

HERE = pathlib.Path(__file__).resolve().parent
os.chdir(HERE / "ore")
(HERE / "ore/Output").mkdir(exist_ok=True)
params = ORE.Parameters()
params.fromFile("ore.xml")
ORE.OREApp(params, True).run()
print("ORE", ORE.__version__ if hasattr(ORE, "__version__") else "", "done: ore/Output")
