#!/usr/bin/env bash
# `just references`: fetch the primary sources into concepts/reference/ (gitignored).
# Existing files are kept, so a re-run only fills gaps. EUR-Lex often answers
# non-browser clients with an empty 202; those files are listed for a browser download.
# The three pinned artefacts come from the justfile (CEN_*, PEPPOL_*, KOSIT_*), which spells each pin once.
set -euo pipefail
ua='Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36'
root=concepts/reference; missing=""
fetch() { # DIR FILE URL [ALT_URL]
    mkdir -p "$root/$1"
    if [ -s "$root/$1/$2" ]; then echo "kept     $1/$2"; return 0; fi
    for url in "$3" "${4:-}"; do
        [ -n "$url" ] || continue
        if curl -fsSL -A "$ua" --retry 3 --retry-delay 5 --max-time 900 -o "$root/$1/$2.part" "$url" && [ -s "$root/$1/$2.part" ]; then
            mv "$root/$1/$2.part" "$root/$1/$2"; echo "fetched  $1/$2"; return 0
        fi
        rm -f "$root/$1/$2.part"
    done
    echo "MISSING  $1/$2  <- $3" >&2; missing="$missing  $1/$2  <- $3"$'\n'
}
fetch standard stn-en-16931-1-a1-en.pdf 'https://www.normoff.gov.sk/files/docs/e-fakturacia-stn-en-16931-1-a1-614d692fbcaa2.pdf'
fetch cen "$CEN_ZIP" "$CEN_URL"
fetch cen "$PEPPOL_ZIP" "$PEPPOL_URL"
fetch cen "$KOSIT_ZIP" "$KOSIT_URL"
fetch cen xrechnung-3.0.2-validator-configuration-2026-08-31.zip 'https://github.com/itplr-kosit/validator-configuration-xrechnung/releases/download/v2026-08-31/xrechnung-3.0.2-validator-configuration-2026-08-31.zip'
fetch cen kosit-validator-1.6.3.zip 'https://github.com/itplr-kosit/validator/releases/download/v1.6.3/validator-1.6.3.zip'
for d in 'vat-directive-2006-112 32006L0112 dir/2006/112' 'late-payment-directive-2011-7 32011L0007 dir/2011/7' \
         'einvoicing-directive-2014-55 32014L0055 dir/2014/55' 'decision-2017-1870-en16931-reference 32017D1870 dec_impl/2017/1870'; do
    set -- $d
    fetch eu "$1-en.pdf" "https://eur-lex.europa.eu/legal-content/EN/TXT/PDF/?uri=CELEX:$2" "https://eur-lex.europa.eu/eli/$3/oj/eng/pdf"
done
for l in ustg_1980/UStG:ustg ustdv_1980/UStDV:ustdv pangv_2022/PAngV:pangv bgb/BGB:bgb erechv/ERechV:erechv eeg_2014/EEG_2023:eeg-2023 stromstg/StromStG:stromstg kav/KAV:kav; do
    fetch law "${l#*:}.pdf" "https://www.gesetze-im-internet.de/${l%:*}.pdf"
done
fetch codelists iso-4217-list-one.xml 'https://www.six-group.com/dam/download/financial-information/data-center/iso-currrency/lists/list-one.xml'
fetch market rusty_money-master.zip 'https://github.com/varunsrin/rusty_money/archive/refs/heads/master.zip'
[ -z "$missing" ] || printf '\nNot fetched (download in a browser, keep the file name):\n%s' "$missing" >&2
