#!/usr/bin/env python3
from argparse import ArgumentDefaultsHelpFormatter, ArgumentParser
from pathlib import Path

import yaml

DEFAULT_CU = 2
CU_METRIC = "subway_rpc_calls_finished"

SOURCE_ROOT = Path(__file__).parent.parent

parser = ArgumentParser(
    description="Generate record rules for a given rpc config",
    formatter_class=ArgumentDefaultsHelpFormatter,
)
parser.add_argument(
    "config",
    type=Path,
    nargs="?",
    default=SOURCE_ROOT / "rpc_configs/geth.yml",
    help="Path to the rpc config file",
)
parser.add_argument(
    "-o",
    "--output",
    type=Path,
    required=False,
    help="Path to the output directory",
)
parser.add_argument(
    "-n",
    "--namespace",
    type=str,
    default="proxies",
    help="Namespace for the generated rules",
)

# parser.add_argument(
#     "--vm",
#     action="store_true",
#     help="Generate VMRules instead of PrometheusRule",
# )

parser.add_argument(
    "--prom",
    action="store_true",
    help="Generate PrometheusRule instead of VMRule",
)


def main():
    args = parser.parse_args()
    cfgPath: Path = args.config
    rpcs = yaml.safe_load(cfgPath.read_text())
    if not args.output:
        args.output = SOURCE_ROOT / "monitoring" / f"rules.cu.{cfgPath.name}"
    output = args.output
    rules = []

    for m in rpcs["methods"]:
        method = m["method"]
        weight = m.get("rate_limit_weight", DEFAULT_CU)
        rules.append(
            {
                "record": "subway_rpc_cu_counter",
                "expr": f'{CU_METRIC}{{method="{method}"}} * {weight}',
            }
        )

    manifest = {
        "apiVersion": "operator.victoriametrics.com/v1beta1",
        "kind": "VMRule",
        "metadata": {
            "name": f"subway-cu-record-{cfgPath.stem}",
            "namespace": args.namespace,
        },
        "spec": {"groups": [{"name": f"subway-cu-record-{cfgPath.stem}.rules", "rules": rules}]},
    }
    if args.prom:
        manifest["apiVersion"] = "monitoring.coreos.com/v1"
        manifest["kind"] = "PrometheusRule"
    # if args.vm:
    #     manifest["apiVersion"] = "operator.victoriametrics.com/v1beta1"
    #     manifest["kind"] = "VMRule"

    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(yaml.dump(manifest))


if __name__ == "__main__":
    main()
