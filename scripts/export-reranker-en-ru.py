"""Builds the reranker directory AURELIUS_RERANK_MODEL points at.

qilowoq/bge-reranker-v2-m3-en-ru is bge-reranker-v2-m3 with the vocabulary cut
to English and Russian: 375M parameters instead of 568M, the transformer
untouched. Exported in half precision it is one 716 MiB model.onnx. On the
control queries (fixtures/eval, 2026-10-08, 37k nodes) it scores exactly as
the full FP32 model does and holds 1.3 GiB of the card instead of 3.5.

The logits leave as float32: fastembed reads the scores as f32.

    uv run --no-project --python 3.12 \
        --with torch --with transformers --with sentencepiece --with protobuf --with onnx \
        python scripts/export-reranker-en-ru.py ~/.local/share/aurelius/models/rerank-bge-v2-m3-en-ru-fp16

Needs a CUDA device: the half-precision graph is traced on the card.
"""

import sys

import torch
from transformers import AutoModelForSequenceClassification, AutoTokenizer

REPO = "qilowoq/bge-reranker-v2-m3-en-ru"


class Scores(torch.nn.Module):
    def __init__(self, model):
        super().__init__()
        self.model = model

    def forward(self, input_ids, attention_mask):
        return self.model(input_ids=input_ids, attention_mask=attention_mask).logits.float()


def main() -> None:
    out = sys.argv[1]
    tok = AutoTokenizer.from_pretrained(REPO)
    tok.save_pretrained(out)
    model = AutoModelForSequenceClassification.from_pretrained(REPO, dtype=torch.float16).cuda().eval()
    model.config.save_pretrained(out)
    batch = tok(["запрос", "query"], ["первый документ", "a second, longer document"], padding=True, return_tensors="pt").to("cuda")
    torch.onnx.export(
        Scores(model),
        (batch["input_ids"], batch["attention_mask"]),
        f"{out}/model.onnx",
        input_names=["input_ids", "attention_mask"],
        output_names=["logits"],
        dynamic_axes={"input_ids": {0: "batch", 1: "tokens"}, "attention_mask": {0: "batch", 1: "tokens"}, "logits": {0: "batch"}},
        opset_version=17,
        dynamo=False,
    )


if __name__ == "__main__":
    main()
