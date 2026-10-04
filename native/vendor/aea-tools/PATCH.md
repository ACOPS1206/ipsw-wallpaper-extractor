# Local patch to aea-tools 0.1.1

The upstream reader guesses whether a decrypted segment is compressed from a
`bvx2` prefix. A stored segment can contain the start of an embedded LZFSE UDIF
stream and must not be decompressed as an independent AEA stream. This reproduces
`bad payload overflow` at cluster 0, segment 0 of 094-84661-378.dmg.aea in the
Apple CDN iPhone19,7 iOS 27.0 (24A437) IPSW: both segment sizes are 1048576 bytes.

The reader now preserves equal-size segments, decodes unequal-size segments
according to the root compression algorithm, and checks decoded length and SHA256
on both initial and dictionary-cached reads. Unsupported algorithms fail explicitly.
Four regression tests cover embedded compression magic, repeated LZFSE decoding,
size/checksum failure, unsupported algorithms and truncated streams.
Original AES/HMAC verification is unchanged. The crate root allows the new chunks_exact_to_as_chunks style lint on upstream
code. All other upstream source files are unchanged. Original MIT and Apache licenses are retained.
