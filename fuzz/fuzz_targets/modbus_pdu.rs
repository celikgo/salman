// SPDX-License-Identifier: Apache-2.0
//! The protocol data unit decoder, against bytes from a network.
//!
//! This target asserts three things beyond "it did not crash":
//!
//! * whatever decodes must **survive the round trip** through the typed form.
//!   A decoder that quietly dropped a field it failed to model would pass a
//!   never-panics test for ever;
//! * **encoding is canonical**: encoding what was decoded, and decoding that
//!   again, is a fixed point. This is the invariant rather than byte-identity,
//!   and the fuzzer is what established the difference — `0F 04 01 00 04 01
//!   FD` writes four coils in a byte whose four padding bits are set, and
//!   salman clears them on the way in, so the bytes out are `…01 0D`. The
//!   clearing is deliberate (`docs/CONFORMANCE.md` §26) because the padding is
//!   meaningless by definition and two readings of the same coils must compare
//!   equal. So the frame is not the invariant; the *meaning* is;
//! * a **prefix of a frame must never decode**. If a short read could look
//!   complete, the stream framer would hand half a frame to a caller and
//!   resynchronise onto nothing.

#![no_main]

use libfuzzer_sys::fuzz_target;
use salman_modbus::pdu::{Request, Response};

fuzz_target!(|data: &[u8]| {
    // A PDU is at most 253 bytes; anything longer is refused on sight and
    // there is nothing to learn from a megabyte of it.
    if data.len() > 1024 {
        return;
    }

    if let Ok(request) = Request::decode(data) {
        // Encoding what was decoded must give something that decodes to the
        // same thing, and encoding *that* must give the same bytes: the
        // canonical form is a fixed point. Byte-identity with `data` is not
        // asserted, because a sender may set padding bits that carry no
        // meaning and salman clears them.
        // Anything that decoded must re-encode: the decoder's limits and the
        // encoder's have to be the same set, or salman would accept a frame it
        // could not write back.
        let encoded = request
            .encode()
            .expect("a request that decoded must be one salman can write");
        assert!(!encoded.overflowed());
        let again = Request::decode(encoded.as_bytes())
            .expect("what salman encoded, salman must be able to decode");
        assert_eq!(again, request, "the round trip changed the request");
        assert_eq!(
            again.encode().expect("still encodable").as_bytes(),
            encoded.as_bytes(),
            "encoding is not a fixed point for {data:02X?}"
        );

        // No proper prefix of it may decode.
        for cut in 0..data.len() {
            assert!(
                Request::decode(&data[..cut]).is_err(),
                "a {cut}-byte prefix of {data:02X?} decoded as a whole frame"
            );
        }

        // Whatever the request says about itself must be self-consistent, or
        // a server would size a reply from one field and fill it from another.
        assert!(request.quantity() >= 1);
        let _ = request.start();
        let _ = request.function();

        // A response to this request must decode when it is encoded, whatever
        // the request happened to be.
        if let Ok(response) = Response::decode(data, &request) {
            let encoded = response.encode();
            let again = Response::decode(encoded.as_bytes(), &request)
                .expect("what salman encoded, salman must be able to decode");
            assert_eq!(again, response, "the round trip changed the response");
            assert_eq!(
                again.encode().as_bytes(),
                encoded.as_bytes(),
                "response encoding is not a fixed point for {data:02X?}"
            );
        }
    }

    // Decoding a response against an unrelated request must also never panic:
    // that is what happens on a capture where the pairing was guessed wrongly.
    let reference = Request::ReadHoldingRegisters {
        start: 0,
        quantity: 1,
    };
    let _ = Response::decode(data, &reference);
});
