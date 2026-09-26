set -e
P="sudo -u postgres psql -qtA -d b2 -v ON_ERROR_STOP=1"
$P <<'SQL'
delete from vpn_leases; delete from node_lease_slots; delete from devices; delete from auth.users; delete from nodes;
insert into nodes values ('e1','h');
insert into auth.users values ('00000000-0000-4000-8000-0000000000d4','x@example.com');
insert into devices (id, account_id,user_id,name) select g::text::uuid, (select account_id from account_members limit 1), '00000000-0000-4000-8000-0000000000d4','d' from (values ('00000000-0000-4000-8000-00000000000a'),('00000000-0000-4000-8000-00000000000b')) v(g);
select agent_sync_lease_slots('e1', jsonb_build_array(jsonb_build_object('slot',0,'generation',1,'valid_until',now()+interval '15 min','credential_ciphertext','\x01','credential_nonce','\x02')));
SQL
A=$(select_acct=1; $P -c "select account_id from account_members limit 1")
( $P -c "begin; select lease_route_slots(null,'00000000-0000-4000-8000-00000000000a','$A','r',array['e1'],600,20,60,600)->>'status'; select pg_sleep(2); commit;" > /tmp/c1 ) &
sleep 0.5
$P -c "select lease_route_slots(null,'00000000-0000-4000-8000-00000000000b','$A','r',array['e1'],600,20,60,600)->>'status'" > /tmp/c2
wait
echo "session1(holds lock 2s): $(grep -v '^$' /tmp/c1 | head -1)  session2(concurrent): $(cat /tmp/c2)"
$P -c "select count(*) from vpn_leases"
