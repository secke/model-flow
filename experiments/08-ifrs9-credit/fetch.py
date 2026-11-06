"""Downloads the Lending Club loan file (loans issued 2007-2018Q4, status as of March 2019) and
writes the 36-month loans issued in 2010-2015 that have reached a final status (fully paid,
charged off or in default) to data/loans.csv, dates as month numbers (12 x year + month - 1).
The data is not committed: run this script first (1.7 GB download, then about a minute)."""
import csv, hashlib, os, sys, urllib.request

URL = "https://huggingface.co/datasets/codesignal/lending-club-loan-accepted/resolve/main/accepted_2007_to_2018Q4.csv"
SHA256 = "3eae03c28fd9d2e8a076ebeb73507e8d4d0f44d90500decdb0936e0933d1f36a"
RAW = "data/accepted.csv"
GRADES = {g: i + 1 for i, g in enumerate("ABCDEFG")}
MONTHS = {m: i + 1 for i, m in enumerate(["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"])}
TERMINAL = {"Fully Paid": 0, "Charged Off": 1, "Default": 1}

os.makedirs("data", exist_ok=True)
if not os.path.exists(RAW):
    urllib.request.urlretrieve(URL, RAW)
h = hashlib.sha256()
with open(RAW, "rb") as f:
    for block in iter(lambda: f.read(1 << 24), b""):
        h.update(block)
if h.hexdigest() != SHA256:
    sys.exit(f"{RAW}: unexpected SHA-256 {h.hexdigest()}")


def month(s):
    m, y = s.split("-")
    return int(y) * 12 + MONTHS[m] - 1


cols = ["vintage", "issue_month", "grade", "funded", "int_rate", "defaulted", "last_payment_month", "total_rec_prncp",
        "recoveries", "collection_recovery_fee"]
w = csv.writer(open("data/loans.csv", "w", newline=""))
w.writerow(cols)
counts = {"kept": 0, "not terminal": 0}
with open(RAW, newline="") as f:
    for r in csv.DictReader(f):
        if r["term"].strip() != "36 months" or not r["issue_d"]:
            continue
        issue = month(r["issue_d"])
        if not 2010 <= issue // 12 <= 2015:
            continue
        status = r["loan_status"].replace("Does not meet the credit policy. Status:", "")
        if status not in TERMINAL:
            counts["not terminal"] += 1
            continue
        last = month(r["last_pymnt_d"]) if r["last_pymnt_d"] else -1
        w.writerow([issue // 12, issue, GRADES[r["grade"]], r["funded_amnt"], float(r["int_rate"]) / 100, TERMINAL[status], last,
                    r["total_rec_prncp"], r["recoveries"], r["collection_recovery_fee"]])
        counts["kept"] += 1
print(counts)
