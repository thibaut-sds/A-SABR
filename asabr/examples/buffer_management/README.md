## Buffer Management

### Run the example

```bash
cargo run --example buffer_management
```

### Context

`BufferManager` tracks the available memory of a node over time. When routing bundles, it ensures that the node's storage capacity is never exceeded during the bundle's retention period (from reception to transmission).

In this example, the contact plan is parsed statically using `BufferManager` for the nodes and `EVLManager` for the contacts, and a full `SpsnHybridParenting` router routes bundles across a 3-node network (`Earth -> Satellite -> Mars`). In the A-SABR contact plan file, the initial volume capacity is provided directly after the node name:
```
node <id> <name> <capacity>
```

### Scenario

The script loads a contact plan from `buffer_format.asabr` where node 1 (Satellite) has a maximum memory capacity of 1000 (`Earth` and `Mars` have large capacities).

`Earth -> Satellite` is available continuously (0..100), while `Satellite -> Mars` has two contacts:

- `60..70` at rate 60 (max volume 600)

- `80..90` at rate 100 (max volume 1000)

Three bundles are routed sequentially from `Earth` to `Mars`, creating asymmetrical, overlapping retention windows on `Satellite`:
- **Bundle 1**: size 600, sent at t=0, uses the first contact (60..70) and is retained on Satellite from t=0 to t=70.
- **Bundle 2**: size 300, sent at t=20, uses the second contact (80..90) and is retained on Satellite from t=20 to t=83 (released after Bundle 1).
- **Bundle 3**: size 250, sent at t=40, attempts to use the remaining volume on the second contact.

Buffer occupancy on `Satellite` :
```
volume
|
| 1150 |             b3 (250) refused
|      |
| 1000 +--------------------------------------------- buffer capacity
|      |
|  900 |       +---------------+
|      |       |    b1 + b2    |
|  600 |   +---+               |
|      |   | b1 only           |
|      |   |                   |
|  300 |   |                   +-------+
|      |   |                   |b2 only|
|    0 +---+---+-------+-------+-------+---+--------> time
           |   |       |       |       |
          t=0 t=20    t=40    t=70    t=83
```

### Behavior

**Bundle 1 (600)** is routed via the first `Satellite -> Mars` contact, saturating its volume and occupying 600 on `Satellite` until t=70.

**Bundle 2 (300)** is routed via the second `Satellite -> Mars` contact, pushing peak memory usage on `Satellite` to 900 between t=20 and t=70, and releasing at t=83.

**Bundle 3 (250)** is rejected (`None`). Although the second `Satellite -> Mars` contact still has enough transmission volume, holding Bundle 3 on `Satellite` from t=40 onward overlaps with both Bundle 1 and Bundle 2 between t=40 and t=70, which would push occupancy to 1150 (> 1000).
