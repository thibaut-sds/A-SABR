use std::fs::File;
use std::io::{BufRead, BufReader};

use a_sabr::bundle::Bundle;
use a_sabr::contact_manager::legacy::evl::EVLManager;
use a_sabr::contact_plan::asabr_file_lexer::parse_from_iter;
use a_sabr::distance::sabr::SABR;
use a_sabr::errors::ASABRError;
use a_sabr::mk_router_from_cp;
use a_sabr::node_manager::buffer::BufferManager;
use generativity::make_guard;

fn main() -> Result<(), ASABRError> {
    let cp = File::open("examples/buffer_management/buffer_format.asabr").unwrap();
    let lines = BufReader::new(cp).lines().map(|l| l.unwrap());

    let contact_plan = parse_from_iter::<BufferManager, EVLManager>(lines)?;

    println!(
        "A-SABR CP parsed, found {} nodes (BufferManager) & {} contacts (EVL)",
        contact_plan.vnodes.len() + contact_plan.realnodes.len(),
        contact_plan.contacts.len()
    );

    make_guard!(id);
    let mut router = mk_router_from_cp!(
        id,
        BufferManager,
        EVLManager,
        1,
        "SpsnHybridParenting",
        contact_plan,
        Some(10),
        SABR
    )?;

    let src = router.node_id_ref(0.into()).unwrap().internal().unwrap();
    router.set_source(src)?;
    let dest = router.node_id_ref(2.into()).unwrap().routable().unwrap();

    println!("\nSatellite has a memory capacity of 1000");

    let bundles = [
        (
            0,
            Bundle {
                priority: 0,
                size: 600,
                expiration: 1000,
            },
        ),
        (
            20,
            Bundle {
                priority: 0,
                size: 300,
                expiration: 1000,
            },
        ),
        (
            40,
            Bundle {
                priority: 0,
                size: 250,
                expiration: 1000,
            },
        ),
    ];

    for (i, (date, bundle)) in bundles.iter().enumerate() {
        println!(
            "\nRouting bundle {} (size {}) from Earth to Mars at t={}:",
            i + 1,
            bundle.size,
            date
        );

        match router.route(dest, *date, bundle, None)? {
            Some((path_output, _first_hop)) => println!("{}", path_output),
            None => println!("No route found (insufficient buffer capacity on Satellite)."),
        }
    }

    // === OUTPUT ===
    // A-SABR CP parsed, found 3 nodes (BufferManager) & 3 contacts (EVL)
    //
    // Satellite has a memory capacity of 1000
    //
    // Routing bundle 1 (size 600) from Earth to Mars at t=0:
    // Route to enode: 2 at t=70 with 2 hop(s):
    // - Reach node enode: 2 at t=70 with 2 hop(s)
    // - Reach node enode: 1 at t=0 with 1 hop(s)
    // - Reach node enode: 0 at t=0 with 0 hop(s)
    //
    //
    // Routing bundle 2 (size 300) from Earth to Mars at t=20:
    // Route to enode: 2 at t=83 with 2 hop(s):
    // - Reach node enode: 2 at t=83 with 2 hop(s)
    // - Reach node enode: 1 at t=20 with 1 hop(s)
    // - Reach node enode: 0 at t=20 with 0 hop(s)
    //
    //
    // Routing bundle 3 (size 250) from Earth to Mars at t=40:
    // No route found (insufficient buffer capacity on Satellite).

    Ok(())
}
