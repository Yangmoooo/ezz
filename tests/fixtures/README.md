# Test fixtures

## `rar-multivolume.partN.rar`

- Source: libarchive test fixture `test_rar_multivolume_single_file.partN.rar.uu`
- Source commit: `33da30bd9c111b897b558e73c4c3f498c680350d`
- Generation: decoded the upstream uuencoded files with `uudecode`
- Password: none
- Expected content: `LibarchiveAddingTest.html`, 20,111 bytes
- License: see `libarchive-LICENSE.txt`

SHA-256:

```text
54769cdaa6aeac494b9ba580804598369ba240f61550620434e08350e4428ec2  rar-multivolume.part1.rar
9dd6b28d2fdf0d012390259548cc86be9535ee6eb5ec630c99fbb5dc8d037ed1  rar-multivolume.part2.rar
7d72ef6e876e98e6a317b7e534c0285615a2953543e68135a8a0dcc94b41cf21  rar-multivolume.part3.rar
```

## `zip-multivolume.z01` and `zip-multivolume.zip`

- Generation tool: Info-ZIP 3.0 (Apple build)
- Generation input: the text `ezz zip volume payload\n` repeated and truncated to 70,000 bytes
- Input timestamp: `2020-01-01 00:00:00`
- Command: `zip -0 -j -s 64k zip-multivolume.zip zip-volume-payload.txt`
- Password: none
- Expected content: `zip-volume-payload.txt`, 70,000 bytes

SHA-256:

```text
1986b2a99c61ee62f8123956da641935b3c00aa51cc91e636c1914ddf3e7d934  zip-multivolume.z01
04d54a723672ae643f9e329fd32547c5f3f73bc6b2648e1aa839abe702be3559  zip-multivolume.zip
```
