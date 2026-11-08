// P5 跨网段连接：地址簿的生成与合并，以及连接建立时的地址记录。
// 注释中文，标识符英文。

use lanmsg_proto::addrbook::{AddressBook, AddressEntry, KnownAddr};
use lanmsg_proto::ids::{DeviceId, UserId};
use lanmsg_proto::types::PathClass;

use crate::error::CoreError;
use crate::net::PeerInfo;
use crate::store::{AddrSource, ContactState, KnownAddrRow, Store};

/// 地址簿只交换局域网与 VPN 地址（v1 不使用公网与中继地址）。
fn exchangeable(p: &PathClass) -> bool {
    matches!(p, PathClass::Lan | PathClass::Vpn)
}

/// 为对端生成地址簿：
/// - 对端是自己的设备：包含所有已知设备（自己的与已接受联系人的，排除已吊销）；
/// - 对端是已接受联系人：只包含自己的设备；
/// - 其余：空。
pub fn build_for(
    store: &Store,
    self_user: &UserId,
    peer_user: &UserId,
) -> Result<AddressBook, CoreError> {
    let mut users = Vec::new();
    if peer_user == self_user {
        users.push(*self_user);
        for c in store.list_contacts(Some(ContactState::Accepted))? {
            users.push(c.user);
        }
    } else if store
        .get_contact(peer_user)?
        .is_some_and(|c| c.state == ContactState::Accepted)
    {
        users.push(*self_user);
    }
    let mut entries = Vec::new();
    for user in users {
        for d in store.devices_of_user(&user)? {
            if d.revoked || store.is_revoked(&d.device)? {
                continue;
            }
            let addrs: Vec<KnownAddr> = store
                .addresses_of(&d.device)?
                .into_iter()
                .filter(|r| exchangeable(&r.path))
                .map(|r| KnownAddr {
                    addr: r.addr.to_string(),
                    path: r.path,
                    last_ok: r.last_ok,
                })
                .collect();
            if !addrs.is_empty() {
                entries.push(AddressEntry {
                    device: d.device,
                    addrs,
                });
            }
        }
    }
    Ok(AddressBook { entries })
}

/// 合并收到的地址簿，返回写入的地址条数。规则见 [`merge_devices`]。
pub fn merge(
    store: &Store,
    self_user: Option<UserId>,
    from: &PeerInfo,
    book: &AddressBook,
    now: i64,
) -> Result<usize, CoreError> {
    Ok(merge_inner(store, self_user, from, book, now)?.0)
}

/// 同 [`merge`]，但返回写入了地址的设备（去重），供调用方唤醒对应的重连任务。
pub fn merge_devices(
    store: &Store,
    self_user: Option<UserId>,
    from: &PeerInfo,
    book: &AddressBook,
    now: i64,
) -> Result<Vec<DeviceId>, CoreError> {
    Ok(merge_inner(store, self_user, from, book, now)?.1)
}

/// 只接受 devices 表中已存在且未吊销的设备；对端不是自己的设备时，只接受属于
/// 对端所属用户的条目（联系人不能借地址簿注入第三方设备）；丢弃公网与中继地址；
/// 写入 `source = Exchanged`，`last_ok` 不超过本机当前时间。
fn merge_inner(
    store: &Store,
    self_user: Option<UserId>,
    from: &PeerInfo,
    book: &AddressBook,
    now: i64,
) -> Result<(usize, Vec<DeviceId>), CoreError> {
    let Some(from_user) = from.cert.as_ref().map(|c| c.user) else {
        return Ok((0, Vec::new()));
    };
    let from_is_mine = self_user == Some(from_user);
    let mut written = 0;
    let mut touched = Vec::new();
    for entry in &book.entries {
        let Some(dev) = store.get_device(&entry.device)? else {
            continue;
        };
        if dev.revoked || store.is_revoked(&entry.device)? {
            continue;
        }
        if !from_is_mine && dev.user != from_user {
            continue;
        }
        let mut any = false;
        for a in &entry.addrs {
            let Ok(addr) = a.addr.parse() else {
                continue;
            };
            if !exchangeable(&a.path) {
                continue;
            }
            store.record_address(&KnownAddrRow {
                device: entry.device,
                addr,
                path: a.path.clone(),
                last_ok: a.last_ok.min(now),
                source: AddrSource::Exchanged,
            })?;
            written += 1;
            any = true;
        }
        if any {
            touched.push(entry.device);
        }
    }
    Ok((written, touched))
}

/// 连接建立时记录对端地址。必须用 `dial_addr`（对端 IP + Hello.listen_port）：
/// 被拨入时 `addr` 是对端的临时源端口，无法回拨。出站连接还会把手动添加的地址绑定到该设备。
pub(crate) fn on_peer_up_record(store: &Store, peer: &PeerInfo, now: i64) -> Result<(), CoreError> {
    if peer.dial_addr.port() != 0 {
        store.record_address(&KnownAddrRow {
            device: peer.device,
            addr: peer.dial_addr,
            path: peer.path.clone(),
            last_ok: now,
            source: AddrSource::Direct,
        })?;
    }
    if peer.outbound {
        store.bind_manual_peer(&peer.addr.to_string(), &peer.device)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{ContactRow, DeviceRow};
    use lanmsg_proto::sign::UserKey;
    use lanmsg_proto::types::{DeviceCert, DeviceType};

    fn cert(user: &UserKey, dev: u8) -> DeviceCert {
        DeviceCert::issue(
            user,
            DeviceId([dev; 32]),
            "d".into(),
            DeviceType::Desktop,
            1,
        )
    }

    fn add_device(store: &Store, c: &DeviceCert) {
        store
            .upsert_device(&DeviceRow {
                device: c.device,
                user: c.user,
                cert: c.clone(),
                name: "d".into(),
                device_type: DeviceType::Desktop,
                last_addr: None,
                last_path: None,
                last_seen: None,
                revoked: false,
            })
            .unwrap();
    }

    fn peer(cert: Option<DeviceCert>, addr: &str, dial: &str, path: PathClass) -> PeerInfo {
        PeerInfo {
            device: cert.as_ref().map(|c| c.device).unwrap_or(DeviceId([0; 32])),
            cert,
            display_name: "p".into(),
            addr: addr.parse().unwrap(),
            dial_addr: dial.parse().unwrap(),
            path,
            caps: vec![],
            outbound: false,
            tls_exporter: [0; 32],
        }
    }

    fn book_for(dev: DeviceId, addr: &str, path: PathClass) -> AddressBook {
        AddressBook {
            entries: vec![AddressEntry {
                device: dev,
                addrs: vec![KnownAddr {
                    addr: addr.into(),
                    path,
                    last_ok: 5,
                }],
            }],
        }
    }

    #[test]
    fn up_records_dial_addr_not_source_port() {
        let store = Store::open_in_memory().unwrap();
        let uk = UserKey::generate();
        let c = cert(&uk, 1);
        let p = peer(Some(c), "10.0.0.2:50123", "10.0.0.2:53320", PathClass::Vpn);
        on_peer_up_record(&store, &p, 100).unwrap();
        let rows = store.addresses_of(&p.device).unwrap();
        assert_eq!(rows[0].addr.to_string(), "10.0.0.2:53320");
        assert_eq!(rows[0].source, AddrSource::Direct);
    }

    #[test]
    fn address_book_from_contact_cannot_inject_third_party() {
        let store = Store::open_in_memory().unwrap();
        let contact = UserKey::generate();
        let other = UserKey::generate();
        let c = cert(&contact, 1);
        let x = cert(&other, 2);
        add_device(&store, &c);
        add_device(&store, &x);
        let from = peer(
            Some(c.clone()),
            "10.0.0.2:1",
            "10.0.0.2:53320",
            PathClass::Vpn,
        );
        let self_user = Some(UserId([9; 32]));
        let n = merge(
            &store,
            self_user,
            &from,
            &book_for(x.device, "10.9.9.9:53320", PathClass::Vpn),
            10,
        )
        .unwrap();
        assert_eq!(n, 0);
        assert!(store.addresses_of(&x.device).unwrap().is_empty());
        // 联系人自己的设备可以接受；公网地址丢弃。
        let ok = merge(
            &store,
            self_user,
            &from,
            &book_for(c.device, "10.0.0.7:53320", PathClass::Vpn),
            10,
        )
        .unwrap();
        assert_eq!(ok, 1);
        let inet = merge(
            &store,
            self_user,
            &from,
            &book_for(c.device, "8.8.8.8:53320", PathClass::Internet),
            10,
        )
        .unwrap();
        assert_eq!(inet, 0);
    }

    #[test]
    fn my_device_may_carry_contact_and_unknown_devices_are_ignored() {
        let store = Store::open_in_memory().unwrap();
        let me = UserKey::generate();
        let contact = UserKey::generate();
        let mine = cert(&me, 1);
        let theirs = cert(&contact, 2);
        add_device(&store, &mine);
        add_device(&store, &theirs);
        let from = peer(Some(mine), "10.0.0.2:1", "10.0.0.2:53320", PathClass::Vpn);
        let mut book = book_for(theirs.device, "10.0.0.7:53320", PathClass::Vpn);
        book.entries
            .extend(book_for(DeviceId([77; 32]), "10.0.0.8:53320", PathClass::Vpn).entries);
        let touched = merge_devices(&store, Some(me.user_id()), &from, &book, 10).unwrap();
        assert_eq!(touched, vec![theirs.device]);
        // last_ok 被夹到本机当前时间。
        let rows = store.addresses_of(&theirs.device).unwrap();
        assert_eq!(rows[0].last_ok, 5);
        assert_eq!(rows[0].source, AddrSource::Exchanged);
    }

    #[test]
    fn build_for_scopes_by_relationship() {
        let store = Store::open_in_memory().unwrap();
        let me = UserKey::generate();
        let friend = UserKey::generate();
        let stranger = UserKey::generate();
        let mine = cert(&me, 1);
        let theirs = cert(&friend, 2);
        add_device(&store, &mine);
        add_device(&store, &theirs);
        for (d, a) in [(&mine, "10.0.0.1:53320"), (&theirs, "10.0.0.2:53320")] {
            store
                .record_address(&KnownAddrRow {
                    device: d.device,
                    addr: a.parse().unwrap(),
                    path: PathClass::Vpn,
                    last_ok: 1,
                    source: AddrSource::Direct,
                })
                .unwrap();
        }
        store
            .upsert_contact_if_newer(&ContactRow {
                user: friend.user_id(),
                state: ContactState::Accepted,
                auto_accept: false,
                ask_above_bytes: None,
                updated: lanmsg_proto::types::Hlc {
                    millis: 1,
                    counter: 0,
                },
            })
            .unwrap();
        let to_mine = build_for(&store, &me.user_id(), &me.user_id()).unwrap();
        assert_eq!(to_mine.entries.len(), 2);
        let to_friend = build_for(&store, &me.user_id(), &friend.user_id()).unwrap();
        assert_eq!(to_friend.entries.len(), 1);
        assert_eq!(to_friend.entries[0].device, mine.device);
        let to_stranger = build_for(&store, &me.user_id(), &stranger.user_id()).unwrap();
        assert!(to_stranger.entries.is_empty());
    }
}
