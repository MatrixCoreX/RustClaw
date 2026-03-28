# Focused regression for read_file summary grounding
# Format: suite|name|tags|prompt

dynamic_guard_focus|explicit_path_two_part_cn|path,file,summary,focus|先读 /home/guagua/test/README.md 开头，再用一句话总结

dynamic_guard_focus|colloquial_en_ultra_read|path,file,en,colloquial,focus|take a quick look at /home/guagua/test/README.md top lines, one-liner only

dynamic_guard_focus|colloquial_cn_ultra_peek_readme|path,file,summary,colloquial,focus|瞅下 /home/guagua/test/README.md 开头，给我一句人话总结

dynamic_guard_focus|english_explicit_readme_summary|path,file,en,focus|Read the beginning of /home/guagua/test/README.md and summarize it in one sentence.
