"""Génère deux portefeuilles synthétiques (développement et revue annuelle) pour un modèle de PD à 7 classes.
Le portefeuille courant glisse vers les classes risquées et la classe 6 y est sous-estimée (DR réel 15 % pour PD 10 %)."""
import random

PD = {1: 0.002, 2: 0.005, 3: 0.01, 4: 0.02, 5: 0.05, 6: 0.10, 7: 0.20}

def sample(n, weights, true_pd, seed):
    rng = random.Random(seed)
    rows = []
    for _ in range(n):
        g = rng.choices(list(PD), weights)[0]
        rows.append(f"{g},{PD[g]},{int(rng.random() < true_pd[g])}")
    return "grade,pd,default\n" + "\n".join(rows) + "\n"

open("data/development.csv", "w").write(sample(8000, [10, 18, 22, 20, 15, 10, 5], PD, 1))
open("data/current.csv", "w").write(sample(8000, [6, 14, 20, 22, 18, 13, 7], {**PD, 6: 0.15}, 2))
