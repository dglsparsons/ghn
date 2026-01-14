import { render } from "@opentui/react/renderer";
import { Text, Box } from "@opentui/react";
import { useInput } from "@opentui/react/hooks";

function App() {
  useInput((input) => {
    if (input.key === "q") process.exit(0);
  });

  return (
    <Box>
      <Text>ghn starting… (press q to quit)</Text>
    </Box>
  );
}

render(<App />);
