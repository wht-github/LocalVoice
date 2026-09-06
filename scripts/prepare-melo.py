"""Remove eager imports of unused languages from the pinned upstream source.

The Chinese/English synthesis algorithm and weights remain upstream's.
This avoids downloading Japanese/French/Spanish tokenizers for a Chinese service.
"""
import ast
import pathlib
import sys

root = pathlib.Path(sys.argv[1]) / "melo"

def replace(relative, old, new):
    path = root / relative
    source = path.read_text()
    if new in source:
        return
    assert old in source, (relative, old)
    path.write_text(source.replace(old, new))

replace("text/cleaner.py", "from . import chinese, japanese, english, chinese_mix, korean, french, spanish",
        "from . import chinese, english, chinese_mix")
path = root / "text/cleaner.py"
source = path.read_text()
start = source.index("language_module_map =")
end = source.index("\ndef clean_text", start)
source = source[:start] + 'language_module_map = {"ZH": chinese, "EN": english, "ZH_MIX_EN": chinese_mix}\n' + source[end:]
path.write_text(source)
# English only needs this pure helper, not the Japanese tokenizer or dictionary.
japanese = (root / "text/japanese.py").read_text()
node = next(n for n in ast.parse(japanese).body if isinstance(n, ast.FunctionDef) and n.name == "distribute_phone")
helper = ast.get_source_segment(japanese, node)
replace("text/english.py", "from .japanese import distribute_phone", helper)
path = root / "text/__init__.py"
source = path.read_text()
source = source[:source.index("def get_bert(")] + '''def get_bert(norm_text, word2ph, language, device):
    if language == "ZH_MIX_EN":
        from .chinese_mix import get_bert_feature
    elif language == "EN":
        from .english_bert import get_bert_feature
    else:
        raise ValueError("This deployment supports Chinese/English only")
    return get_bert_feature(norm_text, word2ph, device)
'''
path.write_text(source)
