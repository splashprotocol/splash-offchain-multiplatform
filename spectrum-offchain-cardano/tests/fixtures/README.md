# Protected royalty parser fixtures

`protected_royalty_v1.hex` and `protected_royalty_v2.hex` are synthetic Conway
transaction outputs. They use the protected pool payment-script hashes from the
Preprod deployment profile, native X, a synthetic Y token, a synthetic pool NFT
and LP token, 50 ADA, 100 million Y units, and one billion circulating LP
units. Their inline datums have zero accrued fees and placeholder **public**
DAO script hashes and royalty public keys. No signing material is present.

The parser tests use these outputs to confirm that the protected V1 and V2
addresses select the intended pool kind and datum layout.
