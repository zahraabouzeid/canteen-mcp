# Canteen MCP Server

A Model Context Protocol server that provides canteen menus from
[OpenMensa](https://openmensa.org). This server enables LLMs to find canteens and
read their meals, prices and opening days.

## Available Tools

- `find_canteens` - Finds canteens by name, city or location.
- `canteen_menu` - Shows the meals of one canteen on one day, with prices and notes.
- `canteen_days` - Lists the days a canteen has published and whether it is closed.

## Usage

Add the server as a custom connector in an MCP client:

`https://canteen.mcp.zahraabouzeid.me/mcp`

No sign-in is needed.

## Data

Menus come from OpenMensa. Allergen notes come from the canteens and may be
incomplete.

## License

This project is licensed under the [AGPL-3.0](LICENSE).
