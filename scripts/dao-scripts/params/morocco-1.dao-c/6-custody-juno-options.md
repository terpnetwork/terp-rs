# custody-juno options (prepared, NOT launched)

Replaces 3-steward-juno-custody-on-juno.json (a DAO on Juno) if chosen.

## A: custody-akash also opens a Juno account
- custody-akash terp10g9vk5kqxv50ezc0sj4n5ves9zxxtu97nfpyctw5zutqdhe2cz5qfgxk3j, proposal 2: empty execute on the Juno note terp180xd...ftrxe
- Juno proxy (predicted): juno1awqp8d4422pz7sm0u45x3q4cve98ufmzlr7wge9mhx7nhp3f2gsqff60dg
- Signer terp10a839738cp8pf5cd00ghthfzyqeusu0lesannt. Page 1: propose+vote. Page 2: execute after the timelock (voting end + 48h, so about 4 days).
- One custody DAO then holds the Akash/Osmosis funds and the Juno funds; steward-akash vetoes both.

## B: new custody-juno subDAO on Terp under steward-juno
- steward-juno terp1a35tgcr4nvqttet76uesqw5fwg55jdpkacp3htm62jxk8xkjvl5qsd4a6z, proposal 1 (signer terp1drrqa2mmk4syy85hrflta73408p0t5n4fkpc8x, no veto, one page):
  wasm instantiate2 of code 23 (dao-dao-core), salt "custody-juno", with the same params as custody-akash (cw4 voting 56 / group 61, proposal-single 24, threshold 1, 48h voting, anyone may propose, 48h veto with veto_before_passed and no early execute), except:
  vetoer = steward-juno core; wasm and dao admin = steward-juno core (or x/gov, to match custody-akash); member = TBD (a funded custody-juno signer).
- custody-juno (predicted): terp12qwtrw4v8adscjjpa5jq0l24nv4uxtnqjv0ev0qgpdmce09my76q4fdn72
- custody-juno proposal 1, ONE proposal with two note executes (Juno note + Osmosis note):
  - Juno proxy juno1zkes0tgj5f3m7w75zqn2ydjrquavcj4k8ykckkm0gw667j2xzjxqu4tw3q
  - Osmosis proxy osmo1uh0uavx5nzdk9gyqh80ey29ygwc5zf5xcj2h87z6cekyal9pffusnem2u2
  - Pages: propose+vote, then execute after about 4 days. Signer = the new member.
