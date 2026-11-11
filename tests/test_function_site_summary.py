"""Counter analysis must use the measured PID and honor the loop analysis gate."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


class FunctionSiteSummaryTests(unittest.TestCase):
    def test_exact_pid_and_selected_loop_variants(self):
        script = Path(__file__).resolve().parents[1] / 'tools/function_site_bench/summarize.py'
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            invocations = []
            for variant, instructions in [('base', 2_000_000_000), ('L0', 2_200_000_000)]:
                for round_number in range(1, 6):
                    run = root / 'runs' / f'tick-{variant}-{round_number}'
                    run.mkdir(parents=True)
                    invocations.append(f'perf\tstat\t-p\t100\t-o\t{run}/perf.csv\t')
                    (run / 'ticks.txt').write_text('# wall_ns 1\n' + '1\n' * 2000)
                    (run / 'perf.csv').write_text(
                        f'horizon-client-100,{instructions},,instructions:u,1,100.00,,\n'
                        'horizon-client-999,999999999999,,instructions:u,1,100.00,,\n')
            # A gated analysis must not open these deliberately invalid unselected results.
            other = root / 'runs/tick-L1-1'
            other.mkdir()
            (other / 'perf.csv').write_text('invalid, unselected counter data')
            (root / 'perf-invocations.txt').write_text('\n'.join(invocations))
            subprocess.run([sys.executable, str(script), str(root), '--variants', 'base,L0'],
                           check=True, capture_output=True, text=True)
            summary = json.loads((root / 'instructions-summary-selected.json').read_text())
            self.assertEqual(set(summary), {'base', 'L0'})
            self.assertEqual(summary['base']['median'], 1_000_000)
            self.assertEqual(summary['L0']['median'], 1_100_000)
            self.assertAlmostEqual(summary['L0']['median_over_base_percent'], 10)
            self.assertFalse((root / 'instructions-summary.json').exists())


if __name__ == '__main__':
    unittest.main()
