# Historical classifier test data

`shorts-health-pre-i83.py.txt` is the unmodified classifier from
`df47737eaca761f9a2a9bb30c91aa7000b98168d`, fetched through GitHub on
2026-10-22. It intentionally misclassifies a tiny automated UMP reply as
EGRESS_REFUSED. It is inert data, not a supported monitor or install payload.

`test-shorts-monitor-install.sh` copies it into temporary storage, asserts
the wrong behavior is reproduced, then proves install/check reject it.
This works with shallow CI checkouts; it must not fall back to an empty file.
