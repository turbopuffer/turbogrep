- [x] Can we use tree-sitter to do the comment parsing in include_precedent_comment? If not, that's fine, just do another look to make the current parser simpler and more robust.
- [x] In `chunker.rs`, add a `hash_chunk_files` which creates chunks without the code content (make this optional). The new workflow in tpuf_sync will then be: hash_chunk_files -> tpuf_chunk_diff -> chunk only files that have changed -> tpuf_apply_diff. We can still do `hash_chunk_files` concurrently with getting all chunks.
- [x] Everywhere should use the same progressbar definition which should be in
lib.rs. I think turbopuffer.rs has the best one right now. use that one always.
repalce all others. e.g. there's one in chunker.rs, it should use that standard
one. One `tg_progress_bar` function in lib.rs that returns a `ProgressBar` and
then use that everywhere.
- [x] When getting the function definition, include into the chunk any comment
preceding the function definition. However, the start_line should preserve being
the line of the function; not the comment. Write a test for this.